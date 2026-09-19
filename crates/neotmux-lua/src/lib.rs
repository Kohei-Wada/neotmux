pub mod api;
pub mod draw;
pub mod ops;
pub mod plugin;
pub mod registry;
pub mod runtime;
pub mod vm;

use std::cell::RefCell;
use std::fs;
use std::rc::Rc;

use mlua::chunk::ChunkMode;
use mlua::{Function, Lua, Value};

use neotmux_core::proto::{DrawCmd, Epoch, PaneId, Rect, Size, StatusFragment};

pub use api::Opts;
pub use api::keymap;

use api::ctx::Ctx;
use draw::Buffers;
use ops::{Level, Op, OpBuffer};
use plugin::{PluginId, Plugins};
use registry::{Kind, KindTag, Registry};
use runtime::Layers;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyOutcome {
    Consumed,
    Fallthrough,
}

#[derive(Default)]
struct Geometry {
    epoch: Epoch,
    size: Size,
    panes: Vec<(PaneId, Rect)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadFailure {
    pub plugin: String,
    pub message: String,
}

pub struct LuaSubsystem {
    lua: Lua,
    layers: Layers,
    plugins: Rc<Plugins>,
    registry: Rc<Registry>,
    ops: Rc<OpBuffer>,
    opts: Rc<RefCell<Opts>>,
    load_failures: Rc<RefCell<Vec<LoadFailure>>>,
    buffers: Rc<Buffers>,
    geometry: RefCell<Geometry>,
}

impl LuaSubsystem {
    pub fn build(layers: Layers) -> anyhow::Result<Self> {
        let lua = vm::new_vm().map_err(|err| anyhow::anyhow!("{err:#}"))?;
        runtime::install(&lua, &layers).map_err(|err| anyhow::anyhow!("{err:#}"))?;

        let plugins = Rc::new(Plugins::new());
        plugin::install_loader(&lua, plugins.clone()).map_err(|err| anyhow::anyhow!("{err:#}"))?;

        let registry = Rc::new(Registry::new());
        let ops = Rc::new(OpBuffer::new());
        let opts = Rc::new(RefCell::new(Opts::default()));
        let load_failures = Rc::new(RefCell::new(Vec::new()));

        api::install(
            &lua,
            plugins.clone(),
            registry.clone(),
            ops.clone(),
            opts.clone(),
            load_failures.clone(),
        )
        .map_err(|err| anyhow::anyhow!("{err:#}"))?;

        Ok(Self {
            lua,
            layers,
            plugins,
            registry,
            ops,
            opts,
            load_failures,
            buffers: Rc::new(Buffers::new()),
            geometry: RefCell::new(Geometry::default()),
        })
    }

    pub fn run_init(&self) -> Vec<LoadFailure> {
        for layer in self.layers.paths() {
            let init_path = layer.join("init.lua");
            let source = match fs::read(&init_path) {
                Ok(source) => source,
                Err(_) => continue,
            };

            let chunk_name = format!("@{}", init_path.display());
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
                        plugin: "init".to_string(),
                        message: format!("{err:#}"),
                    });
                    continue;
                }
            };

            if let Err(err) = func.call::<()>(()) {
                self.load_failures.borrow_mut().push(LoadFailure {
                    plugin: "init".to_string(),
                    message: format!("{err:#}"),
                });
            }
        }

        std::mem::take(&mut *self.load_failures.borrow_mut())
    }

    // `focus` is part of the frozen begin_frame ABI ahead of the task that reads it
    // (pane-aware keymaps / a focus-reading Ctx method); nothing consumes it yet.
    pub fn begin_frame(
        &self,
        epoch: Epoch,
        size: Size,
        panes: Vec<(PaneId, Rect)>,
        _focus: Option<PaneId>,
    ) {
        self.buffers.begin(epoch.clone());
        *self.geometry.borrow_mut() = Geometry { epoch, size, panes };
    }

    pub fn dispatch_tick(&self) {
        let ids = self.registry.snapshot(KindTag::Tick);
        for id in ids {
            let Some(callback) = self.registry.callback(id) else {
                continue;
            };
            let Some(owner) = self.registry.owner(id) else {
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

            self.plugins.push(owner);
            let result = self.lua.scope(|scope| {
                let ctx = scope.create_userdata(ctx)?;
                func.call::<()>(ctx)
            });
            self.plugins.pop();

            match result {
                Ok(()) => self.buffers.commit(owner),
                Err(err) => {
                    self.buffers.discard(owner);
                    let name = self.plugins.name(owner).unwrap_or_default();
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
    }

    pub fn dispatch_key(&self, spec: &str) -> KeyOutcome {
        let Some(normalised) = keymap::normalise(spec) else {
            return KeyOutcome::Fallthrough;
        };

        let prefix = self.opts.borrow().prefix.clone();
        if keymap::normalise(&prefix).as_deref() == Some(normalised.as_str()) {
            return KeyOutcome::Consumed;
        }

        let ids = self.registry.snapshot(KindTag::Keymap);
        let target = ids.into_iter().find(|id| {
            matches!(self.registry.kind(*id), Some(Kind::Keymap(existing)) if existing == normalised)
        });
        let Some(id) = target else {
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

        self.plugins.push(owner);
        let result = func.call::<Value>(());
        self.plugins.pop();

        match result {
            Ok(Value::Boolean(true)) => KeyOutcome::Consumed,
            Ok(_) => KeyOutcome::Fallthrough,
            Err(err) => {
                let name = self.plugins.name(owner).unwrap_or_default();
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

    pub fn frame_output(&self) -> (Vec<DrawCmd>, Vec<StatusFragment>) {
        let epoch = self.geometry.borrow().epoch.clone();
        self.buffers.applicable(&epoch)
    }

    pub fn drain_ops(&self) -> Vec<(PluginId, Op)> {
        self.ops.drain()
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    pub fn plugins(&self) -> &Plugins {
        &self.plugins
    }

    pub fn opt(&self) -> Opts {
        self.opts.borrow().clone()
    }
}
