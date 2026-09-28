use crate::Census;
use crate::CommandInfo;
use crate::CommandOutcome;
use crate::KeyOutcome;
use crate::LoadFailure;
use crate::WriteVerdict;
use crate::api::ctx::Ctx;
use crate::api::{self, Opts, keymap};
use crate::draw::Buffers;
use crate::draw::Floats;
use crate::draw::Reservations;
use crate::host::Ambient;
use crate::host::Clipboard;
use crate::host::Runner;
use crate::host::WindowIds;
use crate::host::{Op, OpBuffer};
use crate::host::{PaneInfos, PaneTtys};
use crate::plugin::Commands;
use crate::plugin::Groups;
use crate::plugin::Layers;
use crate::plugin::{Kind, KindTag, RegistrationId, Registry};
use crate::plugin::{SourceId, Sources};
use crate::sched::Overrun;
use crate::sched::timers::Timers;
use crate::sched::{Budget, Outcome};
use crate::sched::{CountsOverruns, Delivery, Jobs};
use mlua::chunk::ChunkMode;
use mlua::{Function, Lua, Table, Value};
use neotmux_proto::screen::Notice;
use neotmux_proto::screen::PaneRead;
use neotmux_proto::{
    DrawCmd, Epoch, Event, Geometry, Level, PaneId, PaneInfo, PaneTty, Reserved, SessionId, Size,
    WindowId,
};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::fs;
use std::rc::Rc;
use std::time::{Duration, Instant};

pub struct LuaSubsystem {
    lua: Lua,
    layers: Layers,
    sources: Rc<Sources>,
    registry: Rc<Registry>,
    commands: Rc<Commands>,
    ops: Rc<OpBuffer>,
    opts: Rc<RefCell<Opts>>,
    load_failures: Rc<RefCell<Vec<LoadFailure>>>,
    loading: Rc<Cell<bool>>,
    buffers: Rc<Buffers>,
    geometry: Rc<RefCell<Geometry>>,
    pane_info: Rc<RefCell<PaneInfos>>,
    pane_tty: Rc<RefCell<PaneTtys>>,
    /// The notifications the host's surface is showing, as of this frame.
    notices: Rc<RefCell<Vec<Notice>>>,
    /// Which session the work in hand is for, and what the others are holding meanwhile.
    ambient: Rc<Ambient>,
    budget: Rc<Budget>,
    overrun: Rc<Overrun>,
    reservations: Rc<Reservations>,
    window_ids: Rc<WindowIds>,
    floats: Rc<Floats>,
    jobs: Rc<Jobs>,
    timers: Rc<Timers>,
    groups: Rc<Groups>,
    sweep: api::Sweep,
    tick_costs: Rc<crate::sched::TickCosts>,
}

impl LuaSubsystem {
    pub fn build(layers: Layers, panes: Rc<dyn PaneRead>) -> anyhow::Result<Self> {
        Self::build_with(layers, panes, Rc::new(crate::host::Absent))
    }

    /// The half of `build` that takes a clipboard. There is no name here: one layer serves
    /// every session, so `neotmux.session_name()` is a property of the dispatch in hand and
    /// the host names each session as it makes it.
    pub fn build_with(
        layers: Layers,
        panes: Rc<dyn PaneRead>,
        clipboard: Rc<dyn Clipboard>,
    ) -> anyhow::Result<Self> {
        Self::build_with_runner(layers, panes, clipboard, Rc::new(crate::host::NoRunner))
    }

    /// The half of `build_with` that takes a way to start a program. A host that has one
    /// hands it over here; every other caller gets `NoRunner`, which refuses.
    pub fn build_with_runner(
        layers: Layers,
        panes: Rc<dyn PaneRead>,
        clipboard: Rc<dyn Clipboard>,
        runner: Rc<dyn Runner>,
    ) -> anyhow::Result<Self> {
        let lua = crate::plugin::new_vm().map_err(|err| anyhow::anyhow!("{err:#}"))?;
        crate::plugin::install(&lua, &layers).map_err(|err| anyhow::anyhow!("{err:#}"))?;
        let home = layers
            .home()
            .map(|path| path.to_string_lossy().into_owned());

        let sources = Rc::new(Sources::new());
        crate::plugin::install_loader(&lua, sources.clone())
            .map_err(|err| anyhow::anyhow!("{err:#}"))?;

        let registry = Rc::new(Registry::new());
        let commands = Rc::new(Commands::new());
        let opts = Rc::new(RefCell::new(Opts::default()));
        let load_failures = Rc::new(RefCell::new(Vec::new()));
        let geometry = Rc::new(RefCell::new(Geometry::default()));
        let pane_info = Rc::new(RefCell::new(PaneInfos::new()));
        let pane_tty = Rc::new(RefCell::new(PaneTtys::new()));
        let notices = Rc::new(RefCell::new(Vec::new()));
        // Before the op buffer, which reads the bound session at every push.
        let ambient = Rc::new(Ambient::new(
            geometry.clone(),
            pane_info.clone(),
            pane_tty.clone(),
        ));
        let ops = Rc::new(OpBuffer::new(Rc::clone(&ambient)));
        let reservations = Rc::new(Reservations::new());
        let window_ids = Rc::new(WindowIds::new());
        let floats = Rc::new(Floats::new());
        let buffers = Rc::new(Buffers::new());

        let jobs = Rc::new(Jobs::new(runner));
        let timers = Rc::new(Timers::default());
        let overrun = Rc::new(Overrun::new());
        let groups = Rc::new(Groups::default());
        let tick_costs = Rc::new(crate::sched::TickCosts::default());
        let budget = Budget::new(opts.clone());
        Budget::seal(&lua, &budget).map_err(|err| anyhow::anyhow!("{err:#}"))?;

        let sweep = api::Sweep {
            registry: registry.clone(),
            buffers: buffers.clone(),
            reservations: reservations.clone(),
            floats: floats.clone(),
            jobs: jobs.clone(),
            timers: timers.clone(),
            commands: commands.clone(),
            panes: panes.clone(),
        };
        let loading = api::install(
            &lua,
            api::Shared {
                sources: sources.clone(),
                registry: registry.clone(),
                commands: commands.clone(),
                buffers: buffers.clone(),
                ops: ops.clone(),
                opts: opts.clone(),
                load_failures: load_failures.clone(),
                geometry: geometry.clone(),
                pane_info: pane_info.clone(),
                pane_tty: pane_tty.clone(),
                notices: notices.clone(),
                panes,
                clipboard,
                jobs: jobs.clone(),
                timers: timers.clone(),
                overrun: overrun.clone(),
                reservations: reservations.clone(),
                window_ids: window_ids.clone(),
                floats: floats.clone(),
                home,
                ambient: Rc::clone(&ambient),
                groups: groups.clone(),
                sweep: sweep.clone(),
                tick_costs: tick_costs.clone(),
            },
        )
        .map_err(|err| anyhow::anyhow!("{err:#}"))?;

        Ok(Self {
            lua,
            layers,
            sources,
            registry,
            commands,
            ops,
            opts,
            load_failures,
            loading,
            buffers,
            geometry,
            pane_info,
            pane_tty,
            notices,
            ambient,
            budget,
            overrun,
            reservations,
            window_ids,
            floats,
            jobs,
            timers,
            groups,
            sweep,
            tick_costs,
        })
    }

    pub fn run_init(&self) -> Vec<LoadFailure> {
        self.loading.set(true);
        self.arm("the config budget");
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

            // Per layer rather than around the loop: one slow layer must not spend the
            // deadline of the ones after it.
            let budget_ms = self.opts.borrow().config_budget_ms;
            self.budget.enter(Duration::from_millis(budget_ms));
            let called = func.call::<()>(());
            let outcome = self.budget.leave();

            if let Err(err) = called {
                self.load_failures.borrow_mut().push(LoadFailure {
                    source: tag(),
                    message: format!("{err:#}"),
                });
            }
            // Both entries, as a discarded tick gets both: the raised error is what the load
            // hit, and this is what it was measured against. Neither says the other.
            if outcome != Outcome::WithinBudget {
                self.load_failures.borrow_mut().push(LoadFailure {
                    source: tag(),
                    message: format!("loading exceeded its {budget_ms} ms budget"),
                });
            }
        }

        self.budget.disarm(&self.lua);
        self.loading.set(false);
        std::mem::take(&mut *self.load_failures.borrow_mut())
    }

    /// The config, run again in this VM: `unload`, every group emptied, the opts and overrun
    /// strikes back to a start's, every module the layers loaded forgotten, every layer's
    /// `init.lua` run again, and every group nobody named forgotten.
    /// Never called from inside Lua — the host calls it between frames.
    pub fn reload(&self, session: Option<SessionId>) -> Vec<LoadFailure> {
        self.dispatch_unload(session);
        self.sweep.empty(self.groups.begin_reload());
        *self.opts.borrow_mut() = Opts::default();
        self.overrun.forget_strikes();
        match self
            .lua
            .globals()
            .get::<Table>("package")
            .and_then(|p| p.get::<Table>("loaded"))
        {
            Ok(loaded) => {
                for name in self.sources.modules() {
                    if let Err(err) = loaded.set(name, Value::Nil) {
                        self.report(crate::plugin::ANONYMOUS, &err);
                    }
                }
            }
            Err(err) => self.report(crate::plugin::ANONYMOUS, &err),
        }
        // A parked tick is in no group; kept, `resubscribe` would restore the old code's.
        for (id, _) in self.sources.interned() {
            self.overrun.forget_parked(id);
        }
        let failures = self.run_init();
        self.sweep.empty(self.groups.end_reload());
        failures
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

    pub fn begin_frame(&self, session: SessionId, epoch: Epoch, geometry: Geometry) {
        self.buffers.begin(epoch);
        self.republish_geometry(session, geometry);
    }

    /// The layout changed part-way through a frame. What Lua sees catches up at once, so an
    /// event dispatched after it describes the tree as it is. The frame's epoch is the
    /// buffers' own and is not touched here: the tick has already committed under it, and
    /// moving it mid-frame would throw that commit away.
    pub fn republish_geometry(&self, session: SessionId, geometry: Geometry) {
        let _bound = self.ambient.bind(session);
        // Ids the host is naming that this counter did not hand out: a client attaching to a
        // session that was already running learns the watermark here and nowhere else.
        for window in &geometry.windows {
            self.window_ids.seen(window.id);
        }
        *self.geometry.borrow_mut() = geometry;
    }

    /// The notifications still showing, newest first, as the host's surface holds them.
    /// Published rather than asked for, because the surface is the host's: it is what keeps
    /// a notification about the Lua layer itself failing from depending on the Lua layer.
    pub fn publish_notices(&self, notices: Vec<Notice>) {
        *self.notices.borrow_mut() = notices;
    }

    /// What every pane is running, as the host last sampled it. Published on the host's own
    /// cadence rather than per frame: a plugin asks every tick, and the answer is a syscall.
    pub fn publish_pane_info(&self, session: SessionId, info: BTreeMap<PaneId, PaneInfo>) {
        let _bound = self.ambient.bind(session);
        *self.pane_info.borrow_mut() = info;
    }

    /// Where every pane's cursor is and what mode its program has the terminal in, as of the
    /// frame about to be ticked.
    pub fn publish_pane_tty(&self, session: SessionId, tty: BTreeMap<PaneId, PaneTty>) {
        let _bound = self.ambient.bind(session);
        *self.pane_tty.borrow_mut() = tty;
    }

    /// What this session's jobs have to say, before the tick that will draw it. The session is
    /// bound so that an op a callback raises reaches the frame that is running.
    /// Answers whether any callback ran, which is a change the next tick has to draw.
    pub fn dispatch_jobs(&self, session: SessionId) -> bool {
        let limit = self.opts.borrow().job_output_bytes;
        let deliveries = self.jobs.deliver(session, limit);
        if deliveries.is_empty() {
            return false;
        }
        let overrun_limit = self.opts.borrow().tick_overrun_limit;
        self.budgeted(session, || {
            for delivery in deliveries {
                let (job, owner) = (delivery.job(), delivery.owner());
                let measured = self.measured_callback(owner, || self.run_delivery(delivery));
                // An `Exit` names a job that is already out of the map, so an overrun counts for
                // nothing there — which is right: there is no next call to spare.
                self.judge(&*self.jobs, job, owner, measured, overrun_limit);
            }
        });
        true
    }

    /// The timers this session owes a call at `now`, before the tick that will draw what they
    /// changed. The instant is the caller's rather than read here, so that every timer in a
    /// frame is judged against the one moment the frame happened at.
    ///
    /// Answers whether any timer was due, which is a change the next tick has to draw.
    pub fn dispatch_timers(&self, session: SessionId, now: Instant) -> bool {
        let due = self.timers.due(session, now);
        if due.is_empty() {
            return false;
        }
        let overrun_limit = self.opts.borrow().tick_overrun_limit;
        self.budgeted(session, || {
            for timer in due {
                // The whole due list was taken before any of it ran, so a callback that stopped
                // a later timer — or itself — has to be honoured here. Marking it called only
                // after the call is what lets that stop answer that it cancelled something.
                if self.timers.is_stopped(timer.id) {
                    continue;
                }
                let measured = self.measured_callback(timer.owner, || {
                    self.run_callback(timer.owner, &timer.callback, ())
                });
                self.timers.called(timer.id);
                self.judge(
                    &*self.timers,
                    timer.id,
                    timer.owner,
                    measured,
                    overrun_limit,
                );
            }
        });
        true
    }

    /// Runs a batch of job or timer callbacks with `session` bound and the hook armed. Armed
    /// once for the batch, never per callback: `Budget::arm` costs the interpreter a hook for
    /// as long as it is on.
    fn budgeted(&self, session: SessionId, batch: impl FnOnce()) {
        let _bound = self.ambient.bind(session);
        self.arm("the budget");
        batch();
        self.budget.disarm(&self.lua);
    }

    /// What a measured job or timer callback costs the thing it belongs to: forgiven inside its
    /// budget, nothing for a soft overrun, and a strike for a hard one — the last of which
    /// takes it away and says so. `limit` is the caller's, read once for the batch.
    fn judge<S: CountsOverruns>(
        &self,
        sched: &S,
        id: S::Id,
        owner: SourceId,
        measured: Option<bool>,
        limit: u32,
    ) {
        match measured {
            None => sched.forgive(id),
            Some(false) => {}
            Some(true) => {
                if sched.overran(id, limit) {
                    self.notify_stopped(owner, S::WHAT, limit, S::ENDED);
                }
            }
        }
    }

    /// Runs one job or timer callback inside its own budgeted region. `None` is a callback
    /// that stayed inside its budget; otherwise the overrun is reported and the bool says
    /// whether it is severe enough to be held against the callback. A soft overrun counts for
    /// nothing *and forgives nothing*, exactly as `dispatch_tick` treats one: a job that
    /// alternated hard and soft overruns would otherwise never reach its limit.
    ///
    /// Arming is the caller's, once for the whole batch: a region is per callback because the
    /// thing punished for it is per callback, but the hook is not.
    fn measured_callback(&self, owner: SourceId, call: impl FnOnce()) -> Option<bool> {
        let budget_ms = self.opts.borrow().tick_budget_ms;
        self.budget.enter(Duration::from_millis(budget_ms));
        call();
        let outcome = self.budget.leave();
        if outcome == Outcome::WithinBudget {
            return None;
        }
        self.report_text(
            owner,
            format!("a callback exceeded the {budget_ms} ms budget"),
        );
        Some(crate::sched::counts_toward_park(
            outcome,
            Duration::from_millis(budget_ms),
        ))
    }

    /// What the user is told when a runaway job or timer is taken away. Warn, where the
    /// per-callback overrun is logged at error, for the reason `unsubscribe_overrunning`
    /// gives: this is the one part the user has to see. The source is not named in the text —
    /// every sink tags a notify with it already, and `unsubscribe_overrunning` spells it out
    /// only because its name sits inside a `neotmux.resubscribe` the user can copy. Nothing is
    /// parked and nothing is restored: a stopped timer and a killed job do not come back.
    fn notify_stopped(&self, owner: SourceId, what: &str, limit: u32, ended: &str) {
        self.ops.push(
            owner,
            Op::Notify {
                level: Level::Warn,
                text: format!("a {what} exceeded its budget {limit} times and was {ended}"),
            },
        );
    }

    /// One callback, with the arguments its delivery carries. A `None` is passed as `nil`
    /// rather than skipped: `on_exit` is written against four parameters, and dropping the
    /// middle one would shift `err` into `stdout`'s place.
    fn run_delivery(&self, delivery: Delivery) {
        match delivery {
            Delivery::Lines {
                owner,
                callback,
                lines,
                ..
            } => {
                let Ok(lines) = self.lua.create_sequence_from(lines) else {
                    return;
                };
                self.run_callback(owner, &callback, lines);
            }
            Delivery::Exit {
                owner,
                callback,
                code,
                stdout,
                stderr,
                err,
                ..
            } => {
                let Some(callback) = callback else {
                    return;
                };
                self.run_callback(owner, &callback, (code, stdout, stderr, err));
            }
        }
    }

    fn run_callback(
        &self,
        owner: SourceId,
        callback: &mlua::RegistryKey,
        args: impl mlua::IntoLuaMulti,
    ) {
        let Ok(func) = self.lua.registry_value::<Function>(callback) else {
            return;
        };
        if let Err(err) = func.call::<()>(args) {
            self.report(owner, &err);
        }
    }

    /// The unit of commit is the source, not the callback: a source's frame is the union of
    /// what all its tick callbacks drew, and a partial union is not a frame. Committing after
    /// each callback would make the second one's commit replace the first one's entry, so the
    /// first's drawing would never render.
    pub fn dispatch_tick(&self, session: SessionId) {
        let _bound = self.ambient.bind(session);
        // The hook is armed for this frame only: with `LUA_MASKCOUNT` set Lua 5.4 traps on
        // every instruction, so an always-armed hook would halve key and event dispatch too.
        self.arm("the tick budget");
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
            self.budget
                .enter(Duration::from_millis(self.opts.borrow().tick_budget_ms));
            let started = Instant::now();
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
                        geometry.area,
                        geometry.panes.clone(),
                        self.reservations.band(owner),
                        self.floats.by_owner(owner),
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
            self.tick_costs.record(owner, started.elapsed());
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
                if crate::sched::counts_toward_park(outcome, budget)
                    && self.overrun.count(owner, limit)
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
    ///
    /// Among the observers, the owners holding a live float are offered the key before the
    /// rest — topmost float first, registration order inside each owner (spec §5).
    /// One measured region outside a frame: the hook is armed for it and taken away again, for
    /// the reason `dispatch_tick` gives — an always-armed hook halves the interpreter.
    ///
    /// An overrun here is cut off and reported and that is all. Parking takes away a *tick*
    /// subscription and announces "is no longer ticking", and what overran here was not a
    /// tick. The report is anonymous because one region runs callbacks belonging to different
    /// sources; whichever handler raised is named by the error path that ran it.
    fn measured<T>(&self, run: impl FnOnce() -> T) -> T {
        let (value, outcome, budget_ms) = self.region(|_| run());
        if outcome != Outcome::WithinBudget {
            self.report_text(
                crate::plugin::ANONYMOUS,
                format!("a handler exceeded the {budget_ms} ms budget"),
            );
        }
        value
    }

    /// Arms the hook, and says so when it cannot be: `what` names the budget that is off.
    fn arm(&self, what: &str) {
        if let Err(err) = Budget::arm(&self.lua, &self.budget) {
            self.report_text(crate::plugin::ANONYMOUS, format!("{what} is off: {err}"));
        }
    }

    /// One region under the tick budget, armed for it and disarmed after. Answers what `run`
    /// returned, how the region came out, and the budget it was measured against, which `run`
    /// is handed too so a refusal it words itself can name it.
    fn region<T>(&self, run: impl FnOnce(u64) -> T) -> (T, Outcome, u64) {
        self.arm("the budget");
        let budget_ms = self.opts.borrow().tick_budget_ms;
        self.budget.enter(Duration::from_millis(budget_ms));
        let value = run(budget_ms);
        let outcome = self.budget.leave();
        self.budget.disarm(&self.lua);
        (value, outcome, budget_ms)
    }

    pub fn dispatch_key(&self, session: SessionId, spec: &str) -> KeyOutcome {
        let Ok(normalised) = keymap::normalise(spec) else {
            return KeyOutcome::Fallthrough;
        };
        let _bound = self.ambient.bind(session);

        self.measured(|| {
            if self.offer_key(&normalised) == KeyOutcome::Consumed {
                return KeyOutcome::Consumed;
            }
            self.dispatch_binding(&normalised)
        })
    }

    /// Every `on_key` callback, stopping at the first that consumes: the float owners in
    /// float order first, then everyone else in registration order. With no live floats the
    /// order is exactly the plain registration order.
    fn offer_key(&self, normalised: &str) -> KeyOutcome {
        let float_owners: Vec<SourceId> = self.floats.owners_topmost_first();

        for owner in &float_owners {
            for id in self.registry.snapshot(KindTag::Key) {
                if self.registry.owner(id) != Some(*owner) {
                    continue;
                }
                if self.offer_one(id, normalised) {
                    return KeyOutcome::Consumed;
                }
            }
        }

        for id in self.registry.snapshot(KindTag::Key) {
            if let Some(owner) = self.registry.owner(id)
                && float_owners.contains(&owner)
            {
                // already offered above, in float order
                continue;
            }
            if self.offer_one(id, normalised) {
                return KeyOutcome::Consumed;
            }
        }
        KeyOutcome::Fallthrough
    }

    /// One callback, its consume answer, its Lua errors.
    fn offer_one(&self, id: RegistrationId, normalised: &str) -> bool {
        let Some(callback) = self.registry.callback(id) else {
            return false;
        };
        let Some(owner) = self.registry.owner(id) else {
            return false;
        };
        let Ok(func) = self.lua.registry_value::<Function>(&callback) else {
            return false;
        };

        match func.call::<Value>(normalised) {
            Ok(Value::Boolean(true)) => true,
            Ok(_) => false,
            Err(err) => {
                self.report(owner, &err);
                false
            }
        }
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

    /// Whether anything is listening. Answered off the registry, so the door can ask it
    /// without entering Lua — which is what keeps a server with no handler on the path it
    /// has today.
    pub fn has_write_hooks(&self) -> bool {
        !self.registry.snapshot(KindTag::Write).is_empty()
    }

    /// Offers a write to every handler in registration order, stopping at the first refusal.
    ///
    /// Everything that is not a pass is a refusal: a handler that returns `false`, one that
    /// raises, one that overruns its budget, and a table this cannot be built in. A
    /// guardrail that fails open is useless exactly when something is wrong.
    pub fn dispatch_write(
        &self,
        session: SessionId,
        pane: PaneId,
        text: &str,
        from: &str,
    ) -> WriteVerdict {
        let _bound = self.ambient.bind(session);
        let (verdict, outcome, budget_ms) =
            self.region(|budget_ms| self.offer_write(pane, text, from, budget_ms));
        // `offer_write` already names the handler it caught overrunning; this is the
        // backstop for an overrun neither of its checks was running to attribute — nothing
        // was between handlers when the deadline passed.
        if outcome != Outcome::WithinBudget && matches!(verdict, WriteVerdict::Pass) {
            return WriteVerdict::Refused(format!(
                "a write handler exceeded the {budget_ms} ms budget"
            ));
        }
        verdict
    }

    fn offer_write(&self, pane: PaneId, text: &str, from: &str, budget_ms: u64) -> WriteVerdict {
        for id in self.registry.snapshot(KindTag::Write) {
            let Some(callback) = self.registry.callback(id) else {
                return WriteVerdict::Refused("the handler could not be read".to_string());
            };
            let Some(owner) = self.registry.owner(id) else {
                return WriteVerdict::Refused("the handler's owner could not be read".to_string());
            };
            let tag = self.sources.name(owner).unwrap_or_default();
            let Ok(func) = self.lua.registry_value::<Function>(&callback) else {
                return WriteVerdict::Refused(format!("{tag}: the handler could not be read"));
            };
            let write = match self.lua.create_table() {
                Ok(write) => write,
                Err(err) => {
                    return WriteVerdict::Refused(format!("{tag}: {err}"));
                }
            };
            if write.set("pane", pane.get()).is_err()
                || write.set("text", text).is_err()
                || write.set("from", from).is_err()
            {
                return WriteVerdict::Refused(format!("{tag}: the write could not be described"));
            }
            match func.call::<(Value, Value)>(write) {
                Ok(result) => {
                    // Checked after every handler, whatever it answered: a long call inside
                    // it can run past the deadline without the sampling hook ever catching
                    // it mid-flight, and this is the last point that still knows which
                    // handler that was. The budget is the whole dispatch's, so the one named
                    // is the one running when it ran out, not necessarily the slow one.
                    if self.budget.ran_long() {
                        return WriteVerdict::Refused(format!(
                            "{tag}: the {budget_ms} ms budget for this write ran out while it was running"
                        ));
                    }
                    if let (Value::Boolean(false), why) = result {
                        let why = match why {
                            Value::String(why) => why.to_string_lossy().to_string(),
                            _ => format!("refused by {tag}"),
                        };
                        return WriteVerdict::Refused(why);
                    }
                }
                Err(err) => {
                    self.report(owner, &err);
                    return WriteVerdict::Refused(format!("{tag}: {err}"));
                }
            }
        }
        WriteVerdict::Pass
    }

    pub fn dispatch_event(&self, session: SessionId, event: &Event) {
        let _bound = self.ambient.bind(session);
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

        // Measured from here: what is above is the host building a payload, and only what a
        // subscriber does with it is the plugin's time.
        self.measured(|| {
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
        });
    }

    /// The one event the host raises for itself rather than for a pane: the config is about to
    /// run again. Bound to the session that asked, if it is still there.
    pub fn dispatch_unload(&self, session: Option<SessionId>) {
        let _bound = session.map(|session| self.ambient.bind(session));
        let subscribers: Vec<_> = self
            .registry
            .snapshot(KindTag::Event)
            .into_iter()
            .filter(
                |id| matches!(self.registry.kind(*id), Some(Kind::Event(name)) if name == "unload"),
            )
            .collect();
        if subscribers.is_empty() {
            return;
        }
        self.measured(|| {
            for id in subscribers {
                let (Some(callback), Some(owner)) =
                    (self.registry.callback(id), self.registry.owner(id))
                else {
                    continue;
                };
                let Ok(func) = self.lua.registry_value::<Function>(&callback) else {
                    continue;
                };
                let called = self
                    .lua
                    .create_table()
                    .and_then(|fields| func.call::<()>(fields));
                if let Err(err) = called {
                    self.report(owner, &err);
                }
            }
        });
    }

    /// Whether anything is subscribed to an event. The host asks about `pane_output` before
    /// it copies a pane's bytes across a socket for nobody.
    pub fn has_subscriber(&self, event: &str) -> bool {
        self.registry
            .snapshot(KindTag::Event)
            .into_iter()
            .any(|id| matches!(self.registry.kind(id), Some(Kind::Event(name)) if name == event))
    }

    /// Which panes this frame is showing at a place in their own history, and how far back.
    pub fn frame_views(&self) -> BTreeMap<PaneId, usize> {
        self.buffers.applicable_views()
    }

    pub fn frame_output(&self) -> Vec<DrawCmd> {
        let mut out = self.buffers.applicable();
        for cmds in self.buffers.applicable_floats().into_values() {
            out.extend(cmds);
        }
        out
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
    pub fn lay_out_reservations(&self, session: SessionId, surface: Size, applied: Reserved) {
        let _bound = self.ambient.bind(session);
        self.reservations.lay_out(surface, applied);
    }

    /// How many ops are waiting to be drained, for a caller that wants to know whether
    /// something it just offered raised any.
    pub fn ops_raised(&self) -> usize {
        self.ops.mark().0
    }

    /// What was raised for this session, taken out. `None` is the load path: an op raised
    /// before any session existed belongs to none of them.
    pub fn drain_ops(&self, session: Option<SessionId>) -> Vec<(SourceId, Op)> {
        self.ops.drain_for(session)
    }

    /// A session is over: what it was holding is not kept for a session that will never ask
    /// for it again.
    pub fn forget_session(&self, session: SessionId) {
        // `on_exit` runs exactly once for every job that started, whatever ended it, and a
        // session ending is one of the things that can end it: the chunk that wrote the
        // callback is still there, and a plugin holding a lock for a job's lifetime has to
        // hear that the job is over. An op raised in here may reach nobody, since this session
        // is on its way out — the point is the plugin's own bookkeeping, not the screen.
        let endings = self.jobs.end_session(session);
        if !endings.is_empty() {
            let _sweeping = self.jobs.sweeping();
            // And the timers', for the same reason a `start` is refused here: a timer armed
            // from one of these callbacks would be bound to a session with no frame left to
            // fire it, so nothing would ever call it and nothing would ever drop it.
            let _arming = self.timers.sweeping();
            self.budgeted(session, || {
                for delivery in endings {
                    let owner = delivery.owner();
                    self.measured_callback(owner, || self.run_delivery(delivery));
                }
            });
        }
        // A timer is a promise to call again, not a thing owed an ending, so it is dropped
        // where a job is reported.
        self.timers.remove_all_for(session);
        self.ambient.forget(session);
    }

    /// What a session is called, for `neotmux.session_name()` to answer during its dispatches.
    pub fn name_session(&self, session: SessionId, name: &str) {
        self.ambient.name(session, name);
    }

    /// How many clients are attached to `session` this frame, for `neotmux.session.attached`.
    pub fn publish_attached(&self, session: SessionId, clients: usize) {
        self.ambient.set_attached(session, clients);
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// Every registered command, in name order, its owner already resolved to the name
    /// `neotmux.command.list()` would give a plugin — the host caller this is for is the CLI,
    /// which has no other way to say where a command came from.
    pub fn commands(&self) -> Vec<CommandInfo> {
        self.commands
            .list()
            .into_iter()
            .map(|info| CommandInfo {
                name: info.name,
                desc: info.desc,
                source: self.sources.name(info.owner).unwrap_or_default(),
            })
            .collect()
    }

    /// Runs a registered command by name, with `session` bound as the ambient session so the
    /// handler's own calls — `neotmux.window.create()` and the rest — mean that session's, not
    /// whichever one ticked last. Measured under the tick budget like every other callback; an
    /// overrun is a `Failed`, not a hang.
    pub fn run_command(&self, session: SessionId, name: &str, args: &[String]) -> CommandOutcome {
        let _bound = self.ambient.bind(session);
        let Some((callback, owner)) = self.commands.get(name) else {
            return CommandOutcome::Unknown;
        };
        let tag = self.sources.name(owner).unwrap_or_default();

        let (outcome, final_outcome, budget_ms) =
            self.region(|_| self.call_command(&callback, owner, &tag, args));

        // The same backstop `dispatch_write` uses: an overrun neither the hook nor the call's
        // own error path caught is only visible once the region closes, and only a success
        // the budget disagrees with needs overriding — a `Failed` already names what went
        // wrong.
        if final_outcome != Outcome::WithinBudget && matches!(outcome, CommandOutcome::Ran(_)) {
            return CommandOutcome::Failed(format!("{tag}: exceeded the {budget_ms} ms budget"));
        }
        outcome
    }

    fn call_command(
        &self,
        callback: &Rc<mlua::RegistryKey>,
        owner: SourceId,
        tag: &str,
        args: &[String],
    ) -> CommandOutcome {
        let Ok(func) = self.lua.registry_value::<Function>(callback) else {
            return CommandOutcome::Failed(format!("{tag}: the handler could not be read"));
        };
        let table = match self.lua.create_table() {
            Ok(table) => table,
            Err(err) => return CommandOutcome::Failed(format!("{tag}: {err}")),
        };
        for (at, arg) in args.iter().enumerate() {
            if table.set(at + 1, arg.as_str()).is_err() {
                return CommandOutcome::Failed(format!(
                    "{tag}: the arguments could not be described"
                ));
            }
        }

        match func.call::<Value>(table) {
            Ok(value) => match crate::plugin::lines_from(value) {
                Ok(lines) => CommandOutcome::Ran(lines),
                Err(why) => CommandOutcome::Failed(format!("{tag}: {why}")),
            },
            Err(err) => {
                self.report(owner, &err);
                CommandOutcome::Failed(format!("{tag}: {err}"))
            }
        }
    }

    pub fn sources(&self) -> &Sources {
        &self.sources
    }

    /// Every source's live floats, in grant order (spec §2, §3). An accessor like
    /// `sources()`/`registry()`, for the host and for tests: nothing in `dispatch_tick`
    /// reads through it yet — the ctx layer (Task 4) is the first production caller, when a
    /// tick starts drawing into a float.
    pub fn floats(&self) -> &Floats {
        &self.floats
    }

    pub fn opt(&self) -> Opts {
        self.opts.borrow().clone()
    }

    /// What the host is holding for the Lua layer, counted — what a reload is checked against.
    pub fn census(&self) -> Census {
        Census {
            registrations: self.registry.kinds(),
            commands: self.commands.list().into_iter().map(|c| c.name).collect(),
            reserved: self.reservations.totals(),
            floats: self.floats.count(),
            jobs: self.jobs.len(),
            timers: self.timers.len(),
        }
    }

    /// Who holds each band, in reservation order.
    pub fn reservation_owners(&self) -> Vec<SourceId> {
        self.reservations.owners()
    }
}
