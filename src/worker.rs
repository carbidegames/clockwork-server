use std::sync::mpsc::Sender;
use std::sync::Arc;
use crossbeam::sync::MsQueue;
use hyper::{Control, Next, RequestUri};
use webapp::{Application, Request, Responder, BodyResponder};
use webapp::header::Headers;
use webapp::status::StatusCode;
use webapp::method::Method;

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
        _ => "/".into() // TODO: This is a placeholder
    };

    // Build up the request structure
    let request = Request {
        method: token.method(),
        path: path,
        body: token.body().clone(), // TODO: Avoid a copy
    };

    // Send it over to the application
    application.on_request(request, CwResponder {token: token});
}

pub enum WorkerCommand {
    HandleRequest(RequestToken)
}

#[derive(Debug)]
pub enum WorkerResponse {
    Header(StatusCode, Headers),
    Data(Vec<u8>),
    Finish,
}

pub struct RequestToken {
    method: Method,
    uri: RequestUri,
    body: Vec<u8>,
    ctrl: Control,
    sender: Sender<WorkerResponse>,
}

impl RequestToken {
    pub fn new(method: Method, uri: RequestUri, body: Vec<u8>, ctrl: Control, sender: Sender<WorkerResponse>) -> Self {
        RequestToken {
            method: method,
            uri: uri,
            body: body,
            ctrl: ctrl,
            sender: sender,
        }
    }

    fn method(&self) -> Method {
        self.method.clone()
    }

    fn uri(&self) -> &RequestUri {
        &self.uri
    }

    fn body(&self) -> &Vec<u8> {
        &self.body
    }

    fn send_header(&mut self, status_code: StatusCode, headers: Headers) {
        self.sender.send(WorkerResponse::Header(status_code, headers)).unwrap();
        self.ctrl.ready(Next::write()).unwrap();
    }

    fn send_data(&mut self, response_data: Vec<u8>) {
        self.sender.send(WorkerResponse::Data(response_data)).unwrap();
        self.ctrl.ready(Next::write()).unwrap();
    }

    fn finish(self) {
        // TODO: See if we can eliminate the need for a write() and instead directly do an end()
        self.sender.send(WorkerResponse::Finish).unwrap();
        self.ctrl.ready(Next::write()).unwrap();
    }
}

struct CwResponder {
    token: RequestToken
}

impl Responder for CwResponder {
    type R = CwBodyResponder;

    fn start(mut self, status_code: StatusCode, headers: Headers) -> Self::R {
        // Send over the status and headers
        self.token.send_header(status_code, headers);

        // Keep track of the token for the body
        CwBodyResponder {
            token: self.token,
        }
    }
}

struct CwBodyResponder {
    token: RequestToken,
}

impl BodyResponder for CwBodyResponder {
    fn send(&mut self, data: Vec<u8>) {
        self.token.send_data(data);
    }

    fn finish(self) {
        self.token.finish();
    }
}
