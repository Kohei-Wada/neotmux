pub mod api;
pub mod draw;
mod geometry;
pub mod ops;
pub mod registry;
pub mod runtime;
pub mod source;
pub mod vm;

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::fs;
use std::rc::Rc;

use mlua::chunk::ChunkMode;
use mlua::{Function, Lua, Value};

use neotmux_core::proto::{DrawCmd, Epoch, PaneId, Rect, Size, StatusFragment};

pub use api::Opts;
pub use api::event::Event;
pub use api::keymap;

use api::ctx::Ctx;
use draw::Buffers;
use geometry::Geometry;
use ops::{Level, Op, OpBuffer};
use registry::{Kind, KindTag, RegistrationId, Registry};
use runtime::Layers;
use source::{SourceId, Sources};

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
}

impl LuaSubsystem {
    pub fn build(layers: Layers) -> anyhow::Result<Self> {
        let lua = vm::new_vm().map_err(|err| anyhow::anyhow!("{err:#}"))?;
        runtime::install(&lua, &layers).map_err(|err| anyhow::anyhow!("{err:#}"))?;

        let sources = Rc::new(Sources::new());
        source::install_loader(&lua, sources.clone()).map_err(|err| anyhow::anyhow!("{err:#}"))?;

        let registry = Rc::new(Registry::new());
        let ops = Rc::new(OpBuffer::new());
        let opts = Rc::new(RefCell::new(Opts::default()));
        let load_failures = Rc::new(RefCell::new(Vec::new()));
        let geometry = Rc::new(RefCell::new(Geometry::default()));
        let buffers = Rc::new(Buffers::new());

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

    pub fn begin_frame(
        &self,
        epoch: Epoch,
        size: Size,
        panes: Vec<(PaneId, Rect)>,
        focus: Option<PaneId>,
    ) {
        self.buffers.begin(epoch.clone());
        *self.geometry.borrow_mut() = Geometry {
            epoch,
            size,
            panes,
            focus,
        };
    }

    /// The unit of commit is the source, not the callback: a source's frame is the union of
    /// what all its tick callbacks drew, and a partial union is not a frame. Committing after
    /// each callback would make the second one's commit replace the first one's entry, so the
    /// first's drawing would never render.
    pub fn dispatch_tick(&self) {
        let mut by_source: BTreeMap<SourceId, Vec<RegistrationId>> = BTreeMap::new();
        for id in self.registry.snapshot(KindTag::Tick) {
            let Some(owner) = self.registry.owner(id) else {
                continue;
            };
            by_source.entry(owner).or_default().push(id);
        }

        for (owner, ids) in by_source {
            let mut raised = false;
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
                    )
                };

                let result = self.lua.scope(|scope| {
                    let ctx = scope.create_userdata(ctx)?;
                    func.call::<()>(ctx)
                });

                if let Err(err) = result {
                    raised = true;
                    let name = self.sources.name(owner).unwrap_or_default();
                    self.ops.push(
                        owner,
                        Op::Log {
                            level: Level::Error,
                            text: format!("{name}: {err}"),
                        },
                    );
                }
            }

            if raised {
                self.buffers.discard(owner);
            } else {
                self.buffers.commit(owner);
            }
        }
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
                    let name = self.sources.name(owner).unwrap_or_default();
                    self.ops.push(
                        owner,
                        Op::Log {
                            level: Level::Error,
                            text: format!("{name}: {err}"),
                        },
                    );
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
                let name = self.sources.name(owner).unwrap_or_default();
                self.ops.push(
                    owner,
                    Op::Log {
                        level: Level::Error,
                        text: format!("{name}: {err}"),
                    },
                );
                KeyOutcome::Fallthrough
            }
        }
    }

    pub fn dispatch_event(&self, event: &Event) {
        let Ok(bytes) = event.shared_bytes(&self.lua) else {
            return;
        };

        let ids = self.registry.snapshot(KindTag::Event);
        for id in ids {
            let matches = matches!(
                self.registry.kind(id),
                Some(Kind::Event(name)) if name == event.name()
            );
            if !matches {
                continue;
            }
            let Some(callback) = self.registry.callback(id) else {
                continue;
            };
            let Some(owner) = self.registry.owner(id) else {
                continue;
            };
            let Ok(func) = self.lua.registry_value::<Function>(&callback) else {
                continue;
            };
            let Ok(table) = event.table(&self.lua, bytes.as_ref()) else {
                continue;
            };

            if let Err(err) = func.call::<()>(table) {
                let name = self.sources.name(owner).unwrap_or_default();
                self.ops.push(
                    owner,
                    Op::Log {
                        level: Level::Error,
                        text: format!("{name}: {err}"),
                    },
                );
            }
        }
    }

    pub fn frame_output(&self) -> (Vec<DrawCmd>, Vec<StatusFragment>) {
        let epoch = self.geometry.borrow().epoch.clone();
        self.buffers.applicable(&epoch)
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
