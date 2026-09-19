use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use mlua::{HookTriggers, Lua, VmState};

use crate::api::Opts;

/// How often the hook samples the clock. Throughput is flat from 1e3 to 1e6 instructions — a
/// count hook costs a `luaG_traceexec` call per instruction whatever n is — so this is the
/// smallest n that adds no closure cost of its own: measured worst overshoot past an 8 ms
/// budget ~8µs, against ~30µs at 1e4 and ~15% more throughput cost at 1e2.
const SAMPLE_EVERY_N_INSTRUCTIONS: u32 = 1_000;

/// What one source's slice of one frame came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    WithinBudget,
    /// The hook fired past the deadline and unwound the tick.
    Interrupted,
    /// Past the deadline, but no sample ever landed there to cut it off: one long C call
    /// leaves almost no Lua behind it (§5.4). Attributable, not interruptible.
    RanLong {
        elapsed: Duration,
    },
}

#[derive(Clone, Copy)]
struct Active {
    start: Instant,
    budget: Duration,
}

/// What a tick is measured against, and nothing else: counting overruns and acting on them
/// is `crate::overrun`.
pub struct Budget {
    opts: Rc<RefCell<Opts>>,
    active: Cell<Option<Active>>,
    overran: Cell<bool>,
}

impl Budget {
    pub fn new(opts: Rc<RefCell<Opts>>) -> Rc<Self> {
        Rc::new(Self {
            opts,
            active: Cell::new(None),
            overran: Cell::new(false),
        })
    }

    /// Arms the hook for one source's slice of one frame. Outside an `enter`/`leave` pair the
    /// hook does nothing, so key and event dispatch are unmeasured — this is a *tick* budget.
    pub fn enter(&self) {
        self.active.set(self.enabled().then(|| Active {
            start: Instant::now(),
            budget: Duration::from_millis(self.opts.borrow().tick_budget_ms),
        }));
        self.overran.set(false);
    }

    /// Closes the slice and says what it came to. The elapsed comparison is what catches a
    /// tick the hook never got to sample past the deadline.
    pub fn leave(&self) -> Outcome {
        let active = self.active.take();
        if self.overran.get() {
            return Outcome::Interrupted;
        }
        match active {
            Some(active) => {
                let elapsed = active.start.elapsed();
                if elapsed > active.budget {
                    Outcome::RanLong { elapsed }
                } else {
                    Outcome::WithinBudget
                }
            }
            None => Outcome::WithinBudget,
        }
    }

    pub fn overran(&self) -> bool {
        self.overran.get()
    }

    /// Arms the hook for one frame of tick dispatch, and `disarm` takes it away again. It is
    /// scoped rather than left on for the VM's lifetime because `LUA_MASKCOUNT` makes Lua 5.4
    /// set `trap` and route *every* instruction through `luaG_traceexec` whatever n is: an
    /// armed hook costs roughly half the interpreter's throughput, and key handling, event
    /// dispatch and config load are not measured against any budget.
    ///
    /// `set_global_hook`, never `set_hook`: mlua's per-thread hook looks the running thread up
    /// in a weak registry table and silently removes itself when it is missing, so
    /// `coroutine.wrap(function() while true do end end)()` would switch the budget off.
    pub fn arm(lua: &Lua, budget: &Rc<Budget>) -> mlua::Result<()> {
        if !budget.enabled() {
            return Ok(());
        }
        let budget = budget.clone();
        lua.set_global_hook(
            HookTriggers::new().every_nth_instruction(SAMPLE_EVERY_N_INSTRUCTIONS),
            // No Lua calls in here: `luaD_hook` clears `allowhook` for its duration, so
            // anything Lua-side would run unmeasured.
            move |_lua, _debug| {
                let Some(active) = budget.active.get() else {
                    return Ok(VmState::Continue);
                };
                if active.start.elapsed() <= active.budget {
                    return Ok(VmState::Continue);
                }
                budget.overran.set(true);
                Err(mlua::Error::RuntimeError(
                    "tick exceeded its budget".to_string(),
                ))
            },
        )
    }

    pub fn disarm(lua: &Lua) {
        lua.remove_global_hook();
    }
    /// `tick_budget_ms = 0` switches the budget off entirely: no hook, no measurement, no
    /// counting (§5.1). Someone stepping through a plugin under a breakpoint needs that, and
    /// the alternative — every tick overrunning at its first sample — unsubscribes everything.
    pub fn enabled(&self) -> bool {
        self.opts.borrow().tick_budget_ms != 0
    }
}

#[cfg(test)]
mod tests {
    use mlua::Function;

    use super::*;

    fn armed(tick_budget_ms: u64) -> (Lua, Rc<Budget>) {
        let lua = crate::vm::new_vm().unwrap();
        let opts = Rc::new(RefCell::new(Opts {
            tick_budget_ms,
            ..Opts::default()
        }));
        let budget = Budget::new(opts);
        Budget::arm(&lua, &budget).unwrap();
        (lua, budget)
    }

    /// The flag after the loop is what makes "cut off" legible: without it a regression that
    /// let the loop run on would show up as a wedged test run rather than a failure.
    #[test]
    fn a_tick_that_exceeds_its_budget_is_cut_off() {
        let (lua, budget) = armed(5);
        let spin: Function = lua
            .load("reached = false return function() while true do end reached = true end")
            .eval()
            .unwrap();

        budget.enter();
        let result = spin.call::<()>(());
        budget.leave();

        assert!(result.is_err(), "an infinite loop returned normally");
        assert!(budget.overran(), "the overrun was not recorded");
        assert!(
            !lua.globals().get::<bool>("reached").unwrap(),
            "the tick ran past the loop the budget was supposed to cut off"
        );
    }

    /// The global hook is the reason this holds: a per-thread hook would silently not fire on
    /// a thread mlua never registered.
    #[test]
    fn a_coroutine_cannot_escape_the_budget() {
        let (lua, budget) = armed(5);
        let spin: Function = lua
            .load(
                "reached = false
                 return function()
                   coroutine.wrap(function() while true do end reached = true end)()
                 end",
            )
            .eval()
            .unwrap();

        budget.enter();
        let result = spin.call::<()>(());
        budget.leave();

        assert!(result.is_err(), "a coroutine ran past the budget");
        assert!(budget.overran(), "the overrun was not recorded");
        assert!(
            !lua.globals().get::<bool>("reached").unwrap(),
            "the coroutine ran past the loop the budget was supposed to cut off"
        );
    }

    #[test]
    fn outside_a_tick_the_hook_does_nothing() {
        let (lua, budget) = armed(1);
        let loop_ten_million: Function = lua
            .load("return function() local n = 0 for _ = 1, 10000000 do n = n + 1 end return n end")
            .eval()
            .unwrap();

        let result = loop_ten_million.call::<i64>(());

        assert_eq!(result.unwrap(), 10_000_000);
        assert!(!budget.overran());
    }

    #[test]
    fn a_budget_of_zero_arms_no_hook_at_all() {
        let (lua, budget) = armed(0);
        let long: Function = lua
            .load("return function() local n = 0 for _ = 1, 5000000 do n = n + 1 end return n end")
            .eval()
            .unwrap();

        budget.enter();
        let result = long.call::<i64>(());
        budget.leave();

        assert_eq!(result.unwrap(), 5_000_000);
        assert!(!budget.overran(), "a disabled budget recorded an overrun");
    }

    #[test]
    fn disarming_leaves_a_tick_unmeasured() {
        let (lua, budget) = armed(1);
        Budget::disarm(&lua);
        let long: Function = lua
            .load("return function() local n = 0 for _ = 1, 5000000 do n = n + 1 end return n end")
            .eval()
            .unwrap();

        budget.enter();
        let result = long.call::<i64>(());
        budget.leave();

        assert_eq!(result.unwrap(), 5_000_000);
        assert!(!budget.overran(), "a removed hook still fired");
    }

    #[test]
    fn a_tick_inside_its_budget_is_left_alone() {
        let (lua, budget) = armed(5_000);
        let work: Function = lua
            .load("return function() local n = 0 for _ = 1, 100000 do n = n + 1 end return n end")
            .eval()
            .unwrap();

        budget.enter();
        let result = work.call::<i64>(());
        budget.leave();

        assert_eq!(result.unwrap(), 100_000);
        assert!(!budget.overran());
    }
}
