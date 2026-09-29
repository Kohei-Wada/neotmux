//! `neotmux.job`: run a program, hear what it said.

use std::rc::Rc;

use mlua::{Function, Lua, Table, Value};

use crate::api::opt::Opts;
use crate::host::{Ambient, JobSpec};
use crate::plugin::{Groups, Held, Sources};
use crate::sched::{Callbacks, JobId, Jobs};

pub(crate) fn install(
    lua: &Lua,
    jobs: Rc<Jobs>,
    sources: Rc<Sources>,
    ambient: Rc<Ambient>,
    opts: Rc<std::cell::RefCell<Opts>>,
    groups: Rc<Groups>,
) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    let start = lua.create_function(move |lua, (cmd, options): (Table, Option<Table>)| {
        let mut argv = Vec::new();
        for value in cmd.sequence_values::<String>() {
            argv.push(value?);
        }
        if argv.is_empty() {
            return Ok((
                Value::Nil,
                Value::String(lua.create_string("a job needs a command")?),
            ));
        }
        let options = match options {
            Some(table) => table,
            None => lua.create_table()?,
        };
        // None while an `init.lua` is running, which is where a plugin most naturally starts
        // its work: the job is adopted by the first session to run a frame (see `Jobs`).
        let session = ambient.current();
        let spec = JobSpec {
            cmd: argv,
            cwd: options.get::<Option<String>>("cwd")?,
            env: match options.get::<Option<Table>>("env")? {
                Some(env) => {
                    let mut pairs = Vec::new();
                    for entry in env.pairs::<String, String>() {
                        let (key, value) = entry?;
                        pairs.push((key, value));
                    }
                    // A Lua table has no order of its own, so the host would be handed a
                    // different environment every run without this.
                    pairs.sort();
                    pairs
                }
                None => Vec::new(),
            },
        };
        let buffered = options.get::<Option<bool>>("buffered")?.unwrap_or(true);
        let keep = |name: &str| -> mlua::Result<Option<Rc<mlua::RegistryKey>>> {
            Ok(match options.get::<Option<Function>>(name)? {
                Some(f) => Some(Rc::new(lua.create_registry_value(f)?)),
                None => None,
            })
        };
        let callbacks = Callbacks {
            on_stdout: keep("on_stdout")?,
            on_stderr: keep("on_stderr")?,
            on_exit: keep("on_exit")?,
        };
        let owner = crate::plugin::caller(lua, &sources);
        let concurrency = opts.borrow().job_concurrency;
        match jobs.start(spec, owner, session, buffered, callbacks, concurrency) {
            Ok(id) => {
                groups.note(Held::Job(id));
                Ok((Value::Table(handle(lua, Rc::clone(&jobs), id)?), Value::Nil))
            }
            Err(why) => Ok((Value::Nil, Value::String(lua.create_string(&why)?))),
        }
    })?;
    table.set("start", start)?;
    Ok(table)
}

fn handle(lua: &Lua, jobs: Rc<Jobs>, id: JobId) -> mlua::Result<Table> {
    let handle = lua.create_table()?;
    handle.set("id", id.0)?;
    let kill = lua.create_function(move |_, _self: Value| Ok(jobs.kill(id)))?;
    handle.set("kill", kill)?;
    Ok(handle)
}
