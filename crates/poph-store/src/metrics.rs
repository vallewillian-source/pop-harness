use poph_types::{Counter, ExecutionStatus};

#[derive(Debug, Clone)]
pub struct ExecutionMetrics {
    pub started: Counter,
    pub completed: Counter,
    pub abandoned: Counter,
    pub failures: Counter,
    pub rule_errors: Counter,
}

impl ExecutionMetrics {
    pub(crate) fn opened(&self, how_many: u64) {
        if how_many > 0 {
            self.started.increment(how_many);
        }
    }

    pub(crate) fn desfecho(&self, status: ExecutionStatus, how_many: u64) {
        if how_many == 0 {
            return;
        }
        match status {
            ExecutionStatus::Active => {}
            ExecutionStatus::Completed => self.completed.increment(how_many),
            ExecutionStatus::Abandoned => self.abandoned.increment(how_many),
            ExecutionStatus::Failed => self.failures.increment(how_many),
        }
    }

    pub(crate) fn rule_failed(&self, how_many: u64) {
        if how_many > 0 {
            self.rule_errors.increment(how_many);
        }
    }
}
