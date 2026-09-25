/// Budget overruns in a row by one job's or one timer's callbacks. Per job and per timer rather
/// than per source, because what is taken away for them is that job or that timer and not its
/// owner's other work.
#[derive(Default)]
pub struct Strikes(u32);

impl Strikes {
    /// Counts one, answering whether it is the one that reaches `limit`.
    pub fn strike(&mut self, limit: u32) -> bool {
        self.0 += 1;
        self.0 >= limit
    }

    /// A call that stayed inside its budget wipes the slate, the way a tick inside its budget
    /// wipes its source's.
    pub fn forgive(&mut self) {
        self.0 = 0;
    }
}
