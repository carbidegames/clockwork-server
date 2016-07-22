use std::sync::mpsc::Sender;
use std::sync::Arc;
use crossbeam::sync::MsQueue;
use hyper::{Control, Next, RequestUri};
use webapp::{Application, Request, Header, Responder, BodyResponder};

pub fn run_worker<A: Application>(queue: Arc<MsQueue<WorkerCommand>>, application: Arc<A>) {
    // TODO: Catch panics gracefully
    loop {
        match queue.pop() {
            WorkerCommand::HandleRequest(request) => handle_request(request, application.as_ref())
        }
    }
}

fn handle_request<A: Application>(token: RequestToken, application: &A) {
    // TODO: Timeout connections if we receive them X amount of time after they're queued

    // Get the path from the request
    let path = match token.uri() {
        &RequestUri::AbsolutePath(ref path) => path.to_string(),
        other => panic!("Swallowed request uri {:?}, not implemented!", other)
    };

    // Build up the request structure
    let request = Request {
        path: path
    };

    // Send it over to the application
    application.on_request(request, CwResponder {token: token});
}

pub enum WorkerCommand {
    // mpsc is also optimized for oneshot messages, so we use it to return data
    // TODO: Refactor this into a nice wrapper
    HandleRequest(RequestToken)
    //HandleRequest{uri: RequestUri, ctrl: Control, response: Sender<String>}
}

pub struct RequestToken {
    uri: RequestUri,
    ctrl: Control,
    sender: Sender<Vec<u8>>,
}

impl RequestToken {
    pub fn new(uri: RequestUri, ctrl: Control, sender: Sender<Vec<u8>>) -> Self {
        RequestToken {
            uri: uri,
            ctrl: ctrl,
            sender: sender,
        }
    }

    fn uri(&self) -> &RequestUri {
        &self.uri
    }

    fn complete(self, response_data: Vec<u8>) {
        self.sender.send(response_data).unwrap();
        self.ctrl.ready(Next::write()).unwrap();
    }
}

struct CwResponder {
    token: RequestToken
}

impl Responder for CwResponder {
    type R = CwBodyResponder;

    fn start(self, _header: Header) -> Self::R {
        // TODO: Send over the header now instead of waiting until the finish
        CwBodyResponder {
            token: self.token,
            data: Vec::new(),
        }
    }
}

struct CwBodyResponder {
    token: RequestToken,
    data: Vec<u8>,
}

impl BodyResponder for CwBodyResponder {
    fn send(&mut self, mut data: Vec<u8>) {
        // TODO: Send over data immediately instead of waiting until the finish
        self.data.append(&mut data);
    }

    fn finish(self) {
        // TODO: Don't ignore the header data we got
        self.token.complete(self.data);
    }
}
