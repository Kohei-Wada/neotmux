pub mod api;
pub mod budget;
pub mod draw;
mod float;
pub mod ops;
pub mod overrun;
mod paneinfo;
pub mod registry;
pub mod reserve;
pub mod runtime;
pub mod source;
pub mod vm;
pub mod window;

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::fs;
use std::rc::Rc;
use std::time::Duration;

use mlua::chunk::ChunkMode;
use mlua::{Function, Lua, Value};

use neotmux_proto::{
    DrawCmd, Epoch, Event, Geometry, Level, PaneId, PaneInfo, Reserved, Size, WindowId,
};
use neotmux_term::read::PaneRead;

pub use api::Opts;
pub use api::keymap;

use api::ctx::Ctx;
use budget::{Budget, Outcome};
use draw::Buffers;
use float::Floats;
use ops::{Op, OpBuffer};
use overrun::Overrun;
use paneinfo::PaneInfos;
use registry::{Kind, KindTag, RegistrationId, Registry};
use reserve::Reservations;
use runtime::Layers;
use source::{SourceId, Sources};
use window::WindowIds;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyOutcome {
    Consumed,
    Fallthrough,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadFailure {
    pub source: String,
    pub message: String,
}

pub struct LuaSubsystem {
    lua: Lua,
    layers: Layers,
    sources: Rc<Sources>,
    registry: Rc<Registry>,
    ops: Rc<OpBuffer>,
    opts: Rc<RefCell<Opts>>,
    load_failures: Rc<RefCell<Vec<LoadFailure>>>,
    loading: Rc<Cell<bool>>,
    buffers: Rc<Buffers>,
    geometry: Rc<RefCell<Geometry>>,
    pane_info: Rc<RefCell<PaneInfos>>,
    budget: Rc<Budget>,
    overrun: Rc<Overrun>,
    reservations: Rc<Reservations>,
    window_ids: Rc<WindowIds>,
    floats: Rc<Floats>,
}

impl LuaSubsystem {
    pub fn build(layers: Layers, panes: Rc<dyn PaneRead>) -> anyhow::Result<Self> {
        let lua = vm::new_vm().map_err(|err| anyhow::anyhow!("{err:#}"))?;
        runtime::install(&lua, &layers).map_err(|err| anyhow::anyhow!("{err:#}"))?;
        let home = layers
            .home()
            .map(|path| path.to_string_lossy().into_owned());

        let sources = Rc::new(Sources::new());
        source::install_loader(&lua, sources.clone()).map_err(|err| anyhow::anyhow!("{err:#}"))?;

        let registry = Rc::new(Registry::new());
        let ops = Rc::new(OpBuffer::new());
        let opts = Rc::new(RefCell::new(Opts::default()));
        let load_failures = Rc::new(RefCell::new(Vec::new()));
        let geometry = Rc::new(RefCell::new(Geometry::default()));
        let pane_info = Rc::new(RefCell::new(PaneInfos::new()));
        let reservations = Rc::new(Reservations::new());
        let window_ids = Rc::new(WindowIds::new());
        let floats = Rc::new(Floats::new());
        let buffers = Rc::new(Buffers::new());

        let overrun = Rc::new(Overrun::new());
        let budget = Budget::new(opts.clone());

        let loading = api::install(
            &lua,
            api::Shared {
                sources: sources.clone(),
                registry: registry.clone(),
                buffers: buffers.clone(),
                ops: ops.clone(),
                opts: opts.clone(),
                load_failures: load_failures.clone(),
                geometry: geometry.clone(),
                pane_info: pane_info.clone(),
                panes,
                overrun: overrun.clone(),
                reservations: reservations.clone(),
                window_ids: window_ids.clone(),
                floats: floats.clone(),
                home,
            },
        )
        .map_err(|err| anyhow::anyhow!("{err:#}"))?;

        Ok(Self {
            lua,
            layers,
            sources,
            registry,
            ops,
            opts,
            load_failures,
            loading,
            buffers,
            geometry,
            pane_info,
            budget,
            overrun,
            reservations,
            window_ids,
            floats,
        })
    }

    pub fn run_init(&self) -> Vec<LoadFailure> {
        self.loading.set(true);
        for layer in self.layers.paths() {
            let init_path = layer.join("init.lua");
            let source = match fs::read(&init_path) {
                Ok(source) => source,
                Err(_) => continue,
            };

            let chunk_name = format!("@{}", init_path.display());
            // init.lua is a source like any other: interning it here is what gives a load
            // failure a display name, and what makes a call from its top level attribute to
            // the same id the searcher would have produced.
            let id = self.sources.intern(&chunk_name);
            let tag = || self.sources.name(id).unwrap_or_else(|| "init".to_string());

            let func: Function = match self
                .lua
                .load(source)
                .set_name(chunk_name)
                .set_mode(ChunkMode::Text)
                .into_function()
            {
                Ok(func) => func,
                Err(err) => {
                    self.load_failures.borrow_mut().push(LoadFailure {
                        source: tag(),
                        message: format!("{err:#}"),
                    });
                    continue;
                }
            };

            if let Err(err) = func.call::<()>(()) {
                self.load_failures.borrow_mut().push(LoadFailure {
                    source: tag(),
                    message: format!("{err:#}"),
                });
            }
        }

        self.loading.set(false);
        std::mem::take(&mut *self.load_failures.borrow_mut())
    }

    /// One place a raised Lua error becomes an attributed error op. M1b needs to tell an
    /// overrun from a runtime error here.
    fn report(&self, owner: SourceId, err: &mlua::Error) {
        self.report_text(owner, format!("{err}"));
    }

    /// The op carries the source, and every sink prefixes the source's name when it renders
    /// one, so the text must not name it again or the line reads "plugin/x: plugin/x: ...".
    ///
    /// Error, not warn: a frame the plugin author meant to draw was thrown away, and the log
    /// is the only place that says so.
    fn report_text(&self, owner: SourceId, text: String) {
        self.ops.push(
            owner,
            Op::Log {
                level: Level::Error,
                text,
            },
        );
    }

    /// The id the host gives the window it opens itself, before any Lua has run. Taken
    /// from the same counter `window.create()` draws on, so the two can never collide.
    pub fn reserve_window_id(&self) -> WindowId {
        self.window_ids.take()
    }

    pub fn begin_frame(&self, epoch: Epoch, geometry: Geometry) {
        self.buffers.begin(epoch);
        self.republish_geometry(geometry);
    }

    /// The layout changed part-way through a frame. What Lua sees catches up at once, so an
    /// event dispatched after it describes the tree as it is. The frame's epoch is the
    /// buffers' own and is not touched here: the tick has already committed under it, and
    /// moving it mid-frame would throw that commit away.
    pub fn republish_geometry(&self, geometry: Geometry) {
        // Ids the host is naming that this counter did not hand out: a client attaching to a
        // session that was already running learns the watermark here and nowhere else.
        for window in &geometry.windows {
            self.window_ids.seen(window.id);
        }
        *self.geometry.borrow_mut() = geometry;
    }

    /// What every pane is running, as the host last sampled it. Published on the host's own
    /// cadence rather than per frame: a plugin asks every tick, and the answer is a syscall.
    pub fn publish_pane_info(&self, info: BTreeMap<PaneId, PaneInfo>) {
        *self.pane_info.borrow_mut() = info;
    }

    /// The unit of commit is the source, not the callback: a source's frame is the union of
    /// what all its tick callbacks drew, and a partial union is not a frame. Committing after
    /// each callback would make the second one's commit replace the first one's entry, so the
    /// first's drawing would never render.
    pub fn dispatch_tick(&self) {
        // The hook is armed for this frame only: with `LUA_MASKCOUNT` set Lua 5.4 traps on
        // every instruction, so an always-armed hook would halve key and event dispatch too.
        if let Err(err) = Budget::arm(&self.lua, &self.budget) {
            self.report_text(source::ANONYMOUS, format!("the tick budget is off: {err}"));
        }
        let mut by_source: BTreeMap<SourceId, Vec<RegistrationId>> = BTreeMap::new();
        for id in self.registry.snapshot(KindTag::Tick) {
            let Some(owner) = self.registry.owner(id) else {
                continue;
            };
            by_source.entry(owner).or_default().push(id);
        }

        for (owner, ids) in by_source {
            let mut raised = false;
            let mut errors: Vec<String> = Vec::new();
            // Taken before the group runs: a discarded frame takes its ops back out too, so a
            // frame nobody sees does not leave its splits and notifications behind (§5.1).
            let mark = self.ops.mark();
            // The budget is per source per frame, so it spans the whole group: a source that
            // has already spent its slice must not get a fresh one from its next callback.
            self.budget.enter();
            for id in ids {
                let Some(callback) = self.registry.callback(id) else {
                    continue;
                };
                let Ok(func) = self.lua.registry_value::<Function>(&callback) else {
                    continue;
                };

                let ctx = {
                    let geometry = self.geometry.borrow();
                    Ctx::new(
                        owner,
                        self.buffers.clone(),
                        geometry.size,
                        geometry.panes.clone(),
                        self.reservations.band(owner),
                    )
                };

                let result = self.lua.scope(|scope| {
                    let ctx = scope.create_userdata(ctx)?;
                    func.call::<()>(ctx)
                });

                if let Err(err) = result {
                    raised = true;
                    if self.budget.overran() {
                        break;
                    }
                    errors.push(format!("{err}"));
                }
            }
            let outcome = self.budget.leave();
            let (budget_ms, limit) = {
                let opts = self.opts.borrow();
                (opts.tick_budget_ms, opts.tick_overrun_limit)
            };
            let report = match outcome {
                Outcome::WithinBudget => None,
                Outcome::Interrupted => Some(format!("tick exceeded its {budget_ms} ms budget")),
                // What was observed is that no sample landed past the deadline, not where the
                // time went: a long C call and a pure-Lua tick that overshoots inside its last
                // sample window look the same from here.
                Outcome::RanLong { elapsed } => Some(format!(
                    "tick exceeded its {budget_ms} ms budget: {:.1} ms, not cut off — no sample \
                     landed past the deadline",
                    elapsed.as_secs_f64() * 1000.0
                )),
            };
            let overran = report.is_some();

            // An overrun loses the frame even when the tick returned normally: a plugin that
            // `pcall`s around its own loop swallows the budget error (§5.4) and would
            // otherwise keep output the budget says it has lost (§5.1).
            if raised || overran {
                self.buffers.discard(owner);
                self.ops.discard_from(owner, mark);
            } else {
                self.buffers.commit(owner);
            }

            // The host's account of the frame is pushed after the discard, never into it: it
            // is what the plugin author is left with once their own output is gone.
            for text in errors {
                self.report_text(owner, text);
            }
            if let Some(text) = report {
                self.report_text(owner, text);
                let budget = Duration::from_millis(budget_ms);
                if overrun::counts_toward_park(outcome, budget) && self.overrun.count(owner, limit)
                {
                    self.unsubscribe_overrunning(owner, limit);
                }
            } else {
                self.overrun.forgive(owner);
            }
        }
        self.budget.disarm(&self.lua);
    }

    /// The offender keeps its keymaps and event handlers: only what it was measured doing is
    /// taken away, and `neotmux.resubscribe` gives it back (§5.3).
    ///
    /// Notified at warn, where the per-frame overrun is logged at error: this is the one event
    /// the user has to see on screen, and it is recoverable — nothing is broken beyond repair.
    fn unsubscribe_overrunning(&self, owner: SourceId, limit: u32) {
        self.overrun.park(&self.registry, owner);
        api::forget_if_no_longer_drawing(&self.registry, &self.buffers, owner);
        let name = self.sources.name(owner).unwrap_or_default();
        self.ops.push(
            owner,
            Op::Notify {
                level: Level::Warn,
                text: format!(
                    "overran its tick budget {limit} times and is no longer ticking; \
                     neotmux.resubscribe('{name}') restores it"
                ),
            },
        );
    }

    /// Observers first, then the binding (§4.4): an `on_key` callback is the only way to see a
    /// key nobody has bound, which is what a prefix plugin needs to be modal.
    pub fn dispatch_key(&self, spec: &str) -> KeyOutcome {
        let Ok(normalised) = keymap::normalise(spec) else {
            return KeyOutcome::Fallthrough;
        };

        if self.offer_key(&normalised) == KeyOutcome::Consumed {
            return KeyOutcome::Consumed;
        }
        self.dispatch_binding(&normalised)
    }

    /// Every `on_key` callback in registration order, stopping at the first that consumes.
    fn offer_key(&self, normalised: &str) -> KeyOutcome {
        for id in self.registry.snapshot(KindTag::Key) {
            let Some(callback) = self.registry.callback(id) else {
                continue;
            };
            let Some(owner) = self.registry.owner(id) else {
                continue;
            };
            let Ok(func) = self.lua.registry_value::<Function>(&callback) else {
                continue;
            };

            match func.call::<Value>(normalised) {
                Ok(Value::Boolean(true)) => return KeyOutcome::Consumed,
                Ok(_) => continue,
                Err(err) => {
                    self.report(owner, &err);
                }
            }
        }
        KeyOutcome::Fallthrough
    }

    fn dispatch_binding(&self, normalised: &str) -> KeyOutcome {
        let Some(id) = keymap::find(&self.registry, normalised) else {
            return KeyOutcome::Fallthrough;
        };
        let Some(callback) = self.registry.callback(id) else {
            return KeyOutcome::Fallthrough;
        };
        let Some(owner) = self.registry.owner(id) else {
            return KeyOutcome::Fallthrough;
        };
        let Ok(func) = self.lua.registry_value::<Function>(&callback) else {
            return KeyOutcome::Fallthrough;
        };

        match func.call::<Value>(()) {
            Ok(Value::Boolean(true)) => KeyOutcome::Consumed,
            Ok(_) => KeyOutcome::Fallthrough,
            Err(err) => {
                self.report(owner, &err);
                KeyOutcome::Fallthrough
            }
        }
    }

    pub fn dispatch_event(&self, event: &Event) {
        let subscribers: Vec<_> = self
            .registry
            .snapshot(KindTag::Event)
            .into_iter()
            .filter(|id| {
                matches!(
                    self.registry.kind(*id),
                    Some(Kind::Event(name)) if name == event.name()
                )
            })
            .collect();
        // `pane_output` copies the whole chunk into a Lua string, so build the payload only
        // once it is known someone will read it. No subscribers is the default.
        if subscribers.is_empty() {
            return;
        }

        let bytes = match api::event::shared_bytes(event, &self.lua) {
            Ok(bytes) => bytes,
            Err(err) => {
                let name = event.name();
                for id in subscribers {
                    if let Some(owner) = self.registry.owner(id) {
                        self.report_text(owner, format!("{name} payload was not built: {err}"));
                    }
                }
                return;
            }
        };

        for id in subscribers {
            let Some(callback) = self.registry.callback(id) else {
                continue;
            };
            let Some(owner) = self.registry.owner(id) else {
                continue;
            };
            let Ok(func) = self.lua.registry_value::<Function>(&callback) else {
                continue;
            };
            let table = match api::event::table(event, &self.lua, bytes.as_ref()) {
                Ok(table) => table,
                Err(err) => {
                    let name = event.name();
                    self.report_text(owner, format!("{name} payload was not built: {err}"));
                    continue;
                }
            };

            if let Err(err) = func.call::<()>(table) {
                self.report(owner, &err);
            }
        }
    }

    /// Whether anything is subscribed to an event. The host asks about `pane_output` before
    /// it copies a pane's bytes across a socket for nobody.
    pub fn has_subscriber(&self, event: &str) -> bool {
        self.registry
            .snapshot(KindTag::Event)
            .into_iter()
            .any(|id| matches!(self.registry.kind(id), Some(Kind::Event(name)) if name == event))
    }

    pub fn frame_output(&self) -> Vec<DrawCmd> {
        self.buffers.applicable()
    }

    /// What every source has been granted, per edge. Whether the terminal can afford it is
    /// the host's judgement, not this layer's.
    pub fn reserved(&self) -> Reserved {
        self.reservations.totals()
    }

    /// The host has settled the surface: hand every granted reservation its band, out of the
    /// bands the host says it actually applied.
    ///
    /// Called at a frame boundary only. A `neotmux.reserve` or `handle:remove()` from inside
    /// a tick changes what `reserved()` reports, but not the bands this frame is being drawn
    /// against — the same discipline `pane.split` follows, and what keeps a reservation from
    /// invalidating the epoch of a frame already being composed.
    pub fn lay_out_reservations(&self, surface: Size, applied: Reserved) {
        self.reservations.lay_out(surface, applied);
    }

    pub fn drain_ops(&self) -> Vec<(SourceId, Op)> {
        self.ops.drain()
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    pub fn sources(&self) -> &Sources {
        &self.sources
    }

    pub fn opt(&self) -> Opts {
        self.opts.borrow().clone()
    }
}
