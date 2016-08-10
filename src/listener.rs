use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;
use crossbeam::sync::MsQueue;
use hyper::{Control, Decoder, Encoder, Next, RequestUri};
use hyper::net::{HttpStream, HttpListener};
use hyper::server::{Server, Handler, Request, Response};
use webapp::method::Method;
use worker::{WorkerCommand, RequestToken, WorkerResponse};

pub fn run_listener(listener: HttpListener, queue: Arc<MsQueue<WorkerCommand>>) {
    let factory = move |ctrl| {
        let queue = queue.clone();
        HyperHandler::new(ctrl, queue)
    };

    // Set up the server itself
    let server = Server::new(listener)
        .keep_alive(true)
        .idle_timeout(Some(Duration::from_secs(10)))
        .max_sockets(4096);
    let (_listening, server_loop) = server.handle(factory).unwrap();

    // Run the HTTP server loop
    server_loop.run();
}

pub struct HyperHandler {
    // TODO: Consider replacing these Options with a state instead
    method: Option<Method>,
    uri: Option<RequestUri>,
    data: Option<Vec<u8>>,

    ctrl: Option<Control>,
    queue: Arc<MsQueue<WorkerCommand>>,
    receiver: Option<Receiver<WorkerResponse>>,

    responses: Vec<WorkerResponse>,
}

impl HyperHandler {
    pub fn new(ctrl: Control, queue: Arc<MsQueue<WorkerCommand>>) -> Self {
        HyperHandler {
            method: None,
            uri: None,
            data: None,

            ctrl: Some(ctrl),
            queue: queue,
            receiver: None,

            responses: Vec::new(),
        }
    }

    fn recv_into_responses(&mut self) {
        // .pop() needs to return the oldest message
        self.responses.reverse();

        while let Ok(received) = self.receiver.as_ref().unwrap().try_recv() {
            self.responses.push(received);
        }

        self.responses.reverse();
    }
}

impl Handler<HttpStream> for HyperHandler {
    fn on_request(&mut self, req: Request<HttpStream>) -> Next {
        self.method = Some(req.method().clone());
        self.uri = Some(req.uri().clone());
        self.data = Some(Vec::new());
        Next::read()
    }

    fn on_request_readable(&mut self, request: &mut Decoder<HttpStream>) -> Next {
        let done = {
            // Get the data buffer and check what its new size would be
            let data = self.data.as_mut().unwrap();
            let start = data.len();
            let end = start + 4096;

            // Avoid allocating way too much data, we're limiting to 8MB by default
            // TODO: Gracefully handle this with a warning sent to the client
            // TODO: Switch to temp file at 1MB and allow handling more data
            if end > 8388608 + 4096 {
                return Next::end();
            }

            // Read in all the data we have currently available
            data.resize(end, 0);
            match request.try_read(&mut data[start..]) {
                Ok(Some(0)) => {
                    // This means the client has no more data for us
                    data.shrink_to_fit();
                    true
                },
                Ok(Some(n)) => {
                    // Trim away the end
                    data.truncate(start+n);

                    // More data is probably available
                    // TODO: Handle content size header so we can early bail here
                    false
                }
                Ok(None) => {
                    // Trim away the end
                    data.truncate(start);

                    // Why did you even call us? gosh! More data is probably available
                    false
                }
                Err(e) => {
                    error!("read error {:?}", e);
                    return Next::end();
                }
            }
        };

        // Check if we're done
        if done {
            // Queue up a worker task
            // TODO: Refactor task queueing into a nice wrapper
            let (sender, receiver) = mpsc::channel();
            self.receiver = Some(receiver);
            let token = RequestToken::new(
                self.method.take().unwrap(),
                self.uri.take().unwrap(),
                self.data.take().unwrap(),

                self.ctrl.take().unwrap(),
                sender
            );
            self.queue.push(WorkerCommand::HandleRequest(token));

            // We need to wait till we get notified by the worker that we're done
            Next::wait()
        } else {
            Next::read()
        }
    }

    fn on_response(&mut self, response: &mut Response) -> Next {
        self.recv_into_responses();

        // We arrived here after being notified, so we should have the header
        let received = self.responses.pop().unwrap();
        let (status_code, headers) = if let WorkerResponse::Header(s, h) = received {
            (s, h)
        } else {
            panic!("Unexpected worker response {:?}", received);
        };

        // Send the response header
        response.set_status(status_code);
        let rheaders = response.headers_mut();
        *rheaders = headers;

        // If we have any messages left, immediately go on to handle them, if not, wait
        if self.responses.len() != 0 {
            Next::write()
        } else {
            Next::wait()
        }
    }

    fn on_response_writable(&mut self, response: &mut Encoder<HttpStream>) -> Next {
        self.recv_into_responses();

        // Keep handling every bit of data we've received
        while let Some(received) = self.responses.pop() {
            // Make sure we got the correct kind of message we expect here
            let data = match received {
                WorkerResponse::Data(data) => data,
                WorkerResponse::Finish => return Next::end(),
                _ => panic!("Unexpected worker response {:?}", received)
            };

            // Send the data to the client
            response.write(&data).unwrap();
        }

        // The finish gets sent to us externally
        Next::wait()
    }
}
