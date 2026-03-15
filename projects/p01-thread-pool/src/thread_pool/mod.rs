use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{JoinHandle, ThreadId};

pub mod fixed_thread_pool;

type Task = Box<dyn FnOnce() + Send + 'static>;

///
trait ThreadPool {
    fn submit(&self, task: Task);
    fn shutdown(self);
}

pub struct PoolState {
    draining: bool,
    queue: VecDeque<Task>,
    inflight_threads: HashMap<ThreadId, JoinHandle<()>>,
    active_threads: usize,
    current_threads: usize,
}

pub type SharedState = Arc<(Mutex<PoolState>, Condvar)>;

// Lock, grab, unlock, run so that we don't run in a locked environment
pub fn worker(shared_state: SharedState, reserved_threads: usize) {
    loop {
        let task = {
            let (guard, cvar) = &*shared_state;
            let mut state = guard.lock().unwrap();

            loop {

                // Don't run anymore if we're draining/shutting down
                if state.draining && state.queue.is_empty() {
                    state.current_threads -= 1;
                    return;
                }

                if !state.queue.is_empty() {
                    break;
                }
                // Are we a spot thread? Time out instead of waiting forever
                if state.current_threads > reserved_threads {
                    let (new_state, timeout) = cvar
                        .wait_timeout(state, std::time::Duration::from_secs(5))
                        .unwrap();
                    state = new_state;
                    if timeout.timed_out() && state.current_threads > reserved_threads {
                        state.current_threads -= 1;
                        return; // shrink: spot idle thread exits
                    }
                } else {
                    state = cvar.wait(state).unwrap(); // reserved thread waits forever
                }
            }

            // Mark active so we can execute the task
            state.active_threads += 1;
            state.queue.pop_front().unwrap()
        };

        task();

        // Task finished, mark inactive / decrement active count
        let (lock, _) = &*shared_state;
        lock.lock().unwrap().active_threads -= 1;
    }
}