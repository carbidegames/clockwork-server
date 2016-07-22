use std::net::SocketAddr;
use std::thread::{self, JoinHandle};
use std::sync::Arc;
use crossbeam::sync::MsQueue;
use hyper::net::HttpListener;
use webapp::Application;
use listener;
use worker;

pub struct Server<A> {
    application: A,

    worker_threads: usize,
    listener_threads: usize,
}

impl<A: Application> Server<A> {
    pub fn new(application: A) -> Self {
        let cpus = ::num_cpus::get();
        Server {
            application: application,
            worker_threads: cpus,
            listener_threads: cpus,
        }
    }

    pub fn worker_threads(mut self, value: usize) -> Self {
        self.worker_threads = value;
        self
    }

    pub fn listener_threads(mut self, value: usize) -> Self {
        self.listener_threads = value;
        self
    }

    pub fn http(self, addr: &SocketAddr) -> ServerJoinHandle {
        let mut handles = Vec::new();
        let app = Arc::new(self.application);

        // Create the worker threads
        let queue = Arc::new(MsQueue::new());
        for _ in 0..self.worker_threads {
            let queue = queue.clone();
            let app = app.clone();
            let handle = thread::spawn(move || {
                worker::run_worker(queue, app);
            });

            handles.push(handle);
        }

        // Start the listener threads
        let listener = HttpListener::bind(addr).unwrap();
        for _ in 0..self.listener_threads {
            let listener = listener.try_clone().unwrap();
            let queue = queue.clone();
            let handle = thread::spawn(move || {
                listener::run_listener(listener, queue);
            });

            handles.push(handle);
        }

        // Return a handle for the caller to wait on
        ServerJoinHandle {
            handles: handles,
        }
    }
}

/// A handle referring to a server.
pub struct ServerJoinHandle {
    handles: Vec<JoinHandle<()>>,
}

impl ServerJoinHandle {
    pub fn join(self) {
        for handle in self.handles {
            handle.join().unwrap();
        }
    }
}
