pub mod api;
pub mod ops;
pub mod plugin;
pub mod registry;
pub mod runtime;
pub mod vm;

use std::cell::RefCell;
use std::fs;
use std::rc::Rc;

use mlua::chunk::ChunkMode;
use mlua::{Function, Lua};

pub use api::Opts;

use ops::{Op, OpBuffer};
use plugin::{PluginId, Plugins};
use registry::{KindTag, Registry};
use runtime::Layers;

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
}

impl LuaSubsystem {
    pub fn build(layers: Layers) -> anyhow::Result<Self> {
        let lua = vm::new_vm().map_err(|err| anyhow::anyhow!(err.to_string()))?;
        runtime::install(&lua, &layers).map_err(|err| anyhow::anyhow!(err.to_string()))?;

        let plugins = Rc::new(Plugins::new());
        plugin::install_loader(&lua, plugins.clone())
            .map_err(|err| anyhow::anyhow!(err.to_string()))?;

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
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;

        Ok(Self {
            lua,
            layers,
            plugins,
            registry,
            ops,
            opts,
            load_failures,
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
                        message: err.to_string(),
                    });
                    continue;
                }
            };

            if let Err(err) = func.call::<()>(()) {
                self.load_failures.borrow_mut().push(LoadFailure {
                    plugin: "init".to_string(),
                    message: err.to_string(),
                });
            }
        }

        std::mem::take(&mut *self.load_failures.borrow_mut())
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

            self.plugins.push(owner);
            let _ = func.call::<()>(());
            self.plugins.pop();
        }
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
