//! Cooperative `java.lang.Thread` / `Runnable` state for deterministic ports.
//!
//! Constructing a Java thread allocates identity but does not run its target.
//! [`ThreadRuntime::start`] performs the one-shot `NEW -> STARTED` transition,
//! records the host-visible start operation, and queues the thread. The host
//! later calls [`ThreadRuntime::dispatch_next`] to invoke the game-owned
//! `Runnable`; during that callback [`ThreadRuntime::current_thread`] exposes
//! the exact Java thread identity. No native Rust thread is created here.

use std::collections::VecDeque;

use crate::{JavaError, JavaResult};

/// A Java `Thread` reference in the runtime-owned arena.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ThreadId(pub usize);

/// The identity of an object supplied as `new Thread(Runnable)`'s target.
///
/// The constructor also accepts `null`, represented by `Option<RunnableId>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RunnableId(pub usize);

/// The lifecycle observable through this cooperative runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadState {
    New,
    Started,
    Terminated,
}

/// An ordered operation for the host scheduler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostThreadOp {
    Start {
        thread: ThreadId,
        target: Option<RunnableId>,
    },
}

#[derive(Debug)]
struct ThreadCell {
    target: Option<RunnableId>,
    state: ThreadState,
}

/// Runtime-owned Java thread identities and their deterministic start queue.
#[derive(Debug, Default)]
pub struct ThreadRuntime {
    threads: Vec<ThreadCell>,
    pending: VecDeque<ThreadId>,
    host_ops: Vec<HostThreadOp>,
    current: Option<ThreadId>,
}

impl ThreadRuntime {
    pub fn new() -> Self {
        Self::default()
    }

    /// `new Thread(target)` -- allocate a fresh thread in the `NEW` state.
    /// A null target is legal; its default `Thread.run()` does nothing.
    pub fn new_thread(&mut self, target: Option<RunnableId>) -> ThreadId {
        let thread = ThreadId(self.threads.len());
        self.threads.push(ThreadCell {
            target,
            state: ThreadState::New,
        });
        thread
    }

    pub fn state(&self, thread: ThreadId) -> JavaResult<ThreadState> {
        Ok(self.cell(thread)?.state)
    }

    pub fn target(&self, thread: ThreadId) -> JavaResult<Option<RunnableId>> {
        Ok(self.cell(thread)?.target)
    }

    /// `Thread.currentThread()` while a queued Runnable is being dispatched.
    /// Outside cooperative dispatch there is no runtime-owned current thread.
    pub const fn current_thread(&self) -> Option<ThreadId> {
        self.current
    }

    /// `Thread.start()` -- queue a fresh thread exactly once.
    ///
    /// The state is published before the host operation, matching Java's rule
    /// that the newly scheduled Runnable observes an already-started thread.
    pub fn start(&mut self, thread: ThreadId) -> JavaResult<()> {
        let target = {
            let cell = self.cell_mut(thread)?;
            if cell.state != ThreadState::New {
                return Err(JavaError::IllegalThreadState);
            }
            cell.state = ThreadState::Started;
            cell.target
        };
        self.pending.push_back(thread);
        self.host_ops.push(HostThreadOp::Start { thread, target });
        Ok(())
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn host_ops(&self) -> &[HostThreadOp] {
        &self.host_ops
    }

    /// Take the ordered host-operation log without consuming queued work.
    pub fn drain_host_ops(&mut self) -> Vec<HostThreadOp> {
        std::mem::take(&mut self.host_ops)
    }

    /// Cooperatively run the oldest started thread.
    ///
    /// A null target completes without invoking `run`. A non-null target is
    /// passed to the host callback together with its thread identity. The
    /// callback may inspect [`current_thread`](Self::current_thread), start more
    /// threads, or recursively dispatch queued work. Normal completion and a
    /// returned error both terminate this thread and restore the previous
    /// current-thread identity; the error is returned to the host for policy or
    /// logging and is never delivered back to the thread that called `start`.
    pub fn dispatch_next<Run, E>(&mut self, run: Run) -> Result<Option<ThreadId>, E>
    where
        Run: FnOnce(&mut Self, ThreadId, RunnableId) -> Result<(), E>,
    {
        let Some(thread) = self.pending.pop_front() else {
            return Ok(None);
        };
        let target = self
            .cell(thread)
            .expect("only runtime-owned started threads enter the dispatch queue")
            .target;
        let previous = self.current.replace(thread);
        let result = match target {
            Some(target) => run(self, thread, target),
            None => Ok(()),
        };
        self.current = previous;
        self.cell_mut(thread)
            .expect("a queued thread remains in its runtime arena")
            .state = ThreadState::Terminated;
        result.map(|()| Some(thread))
    }

    fn cell(&self, thread: ThreadId) -> JavaResult<&ThreadCell> {
        self.threads.get(thread.0).ok_or(JavaError::NullPointer)
    }

    fn cell_mut(&mut self, thread: ThreadId) -> JavaResult<&mut ThreadCell> {
        self.threads.get_mut(thread.0).ok_or(JavaError::NullPointer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_is_one_shot_and_preserves_fifo_host_order() {
        let mut runtime = ThreadRuntime::new();
        let first = runtime.new_thread(Some(RunnableId(41)));
        let second = runtime.new_thread(None);

        assert_eq!(runtime.state(first), Ok(ThreadState::New));
        assert_eq!(runtime.target(second), Ok(None));
        runtime.start(second).unwrap();
        runtime.start(first).unwrap();
        assert_eq!(runtime.state(second), Ok(ThreadState::Started));
        assert_eq!(runtime.start(second), Err(JavaError::IllegalThreadState));
        assert_eq!(
            runtime.drain_host_ops(),
            vec![
                HostThreadOp::Start {
                    thread: second,
                    target: None,
                },
                HostThreadOp::Start {
                    thread: first,
                    target: Some(RunnableId(41)),
                },
            ]
        );
        assert!(runtime.has_pending());
    }

    #[test]
    fn dispatch_exposes_current_identity_and_restores_it_after_nesting() {
        let mut runtime = ThreadRuntime::new();
        let outer = runtime.new_thread(Some(RunnableId(10)));
        let inner = runtime.new_thread(Some(RunnableId(20)));
        runtime.start(outer).unwrap();
        runtime.start(inner).unwrap();

        let mut seen = Vec::new();
        let dispatched = runtime
            .dispatch_next(|runtime, thread, target| {
                seen.push((thread, target, runtime.current_thread()));
                assert_eq!(runtime.state(thread), Ok(ThreadState::Started));
                let nested = runtime.dispatch_next(|runtime, thread, target| {
                    seen.push((thread, target, runtime.current_thread()));
                    Ok::<_, ()>(())
                })?;
                assert_eq!(nested, Some(inner));
                assert_eq!(runtime.current_thread(), Some(outer));
                Ok::<_, ()>(())
            })
            .unwrap();

        assert_eq!(dispatched, Some(outer));
        assert_eq!(
            seen,
            vec![
                (outer, RunnableId(10), Some(outer)),
                (inner, RunnableId(20), Some(inner)),
            ]
        );
        assert_eq!(runtime.current_thread(), None);
        assert_eq!(runtime.state(inner), Ok(ThreadState::Terminated));
        assert_eq!(runtime.state(outer), Ok(ThreadState::Terminated));
        assert!(!runtime.has_pending());
    }

    #[test]
    fn null_target_terminates_without_invoking_a_runnable() {
        let mut runtime = ThreadRuntime::new();
        let thread = runtime.new_thread(None);
        runtime.start(thread).unwrap();

        let result = runtime.dispatch_next(|_, _, _| -> Result<(), ()> {
            panic!("a null Thread target must not invoke Runnable.run")
        });
        assert_eq!(result, Ok(Some(thread)));
        assert_eq!(runtime.state(thread), Ok(ThreadState::Terminated));
        assert_eq!(runtime.current_thread(), None);
    }

    #[test]
    fn dispatch_failure_still_terminates_and_restores_current_thread() {
        let mut runtime = ThreadRuntime::new();
        let thread = runtime.new_thread(Some(RunnableId(7)));
        runtime.start(thread).unwrap();

        let result = runtime.dispatch_next(|runtime, dispatched, target| {
            assert_eq!(dispatched, thread);
            assert_eq!(target, RunnableId(7));
            assert_eq!(runtime.current_thread(), Some(thread));
            Err("uncaught Runnable failure")
        });
        assert_eq!(result, Err("uncaught Runnable failure"));
        assert_eq!(runtime.state(thread), Ok(ThreadState::Terminated));
        assert_eq!(runtime.current_thread(), None);
    }
}
