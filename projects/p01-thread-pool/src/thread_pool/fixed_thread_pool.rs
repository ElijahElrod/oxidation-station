use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

use super::{PoolState, SharedState, Task, ThreadPool, worker};

pub struct FixedThreadPool {
    reserved_threads: usize,
    max_threads: usize,
    internal_state: SharedState,
}


impl FixedThreadPool {
    fn new(init_threads: usize, max_threads: usize) -> Self {
        let pool_state = PoolState {
            draining: false,
            queue: VecDeque::new(),
            inflight_threads: HashMap::new(),
            active_threads: 0,
            current_threads: init_threads,
        };


        let internal_state = Arc::new((Mutex::new(pool_state), Condvar::new()));

        for _ in 0..init_threads {
            let state_clone = Arc::clone(&internal_state);
            let handle = std::thread::spawn(move || { worker(state_clone, max_threads) });
            let (state, _) = &*internal_state;
            state.lock().unwrap().inflight_threads.insert(handle.thread().id(), handle);
        }

        FixedThreadPool {
            reserved_threads: init_threads,
            max_threads,
            internal_state,
        }
    }
}

impl ThreadPool for FixedThreadPool {
    fn submit(&self, task: Task) {
        let (guard, cvar) = &*self.internal_state;
        let mut state = guard.lock().unwrap();

        if state.draining {
            return;
        }

        state.queue.push_back(task);

        let all_busy = state.active_threads == state.current_threads;
        let can_grow = state.current_threads < state.max_threads;
        // We can size up the threads if everyone is busy but we have room
        if all_busy && can_grow {
            state.current_threads += 1;
            let state_clone = Arc::clone(&self.internal_state);
            let reserved_count = self.reserved_threads;
            let handle = std::thread::spawn(move || { worker(state_clone, reserved_count) });
            state.inflight_threads.insert(handle.thread().id(), handle);
        }

        cvar.notify_one();
    }

    fn shutdown(self) {
        let (guard, cvar) = &*self.internal_state;
        let handles = {
            let mut state = guard.lock().unwrap();
            state.draining = true;
            cvar.notify_all();

            state.inflight_threads.drain()
                .map(|(_, handle)| handle)
                .collect::<Vec<JoinHandle<()>>>()
        };

        for handle in handles {
            handle.join().expect("Panic during shutdown");
        }
    }
}

