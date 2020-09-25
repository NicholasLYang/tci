#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(incomplete_features)]

#[macro_use]
mod util;

#[macro_use]
mod net_io;

mod assembler;
mod ast;
mod buckets;
mod filedb;
mod interpreter;
mod lexer;
mod parser;
mod runtime;
mod type_checker;

#[cfg(test)]
mod test;

use codespan_reporting::term::termcolor::{ColorChoice, StandardStream, WriteColor};
use core::mem;
use embedded_websocket::{HttpHeader, WebSocketReceiveMessageType, WebSocketSendMessageType};
use filedb::FileDb;
use interpreter::Program;
use net_io::WebServerError;
use runtime::{DefaultIO, RuntimeIO};
use std::sync::Mutex;
use util::*;

fn compile<'a>(env: &mut FileDb<'a>) -> Result<Program<'static>, Vec<Error>> {
    let mut buckets = buckets::BucketList::with_capacity(2 * env.size());
    let mut buckets_begin = buckets;
    let mut tokens = lexer::TokenDb::new();
    let mut asts = parser::AstDb::new();
    let mut errors: Vec<Error> = Vec::new();

    let files_list = env.vec();
    let files = files_list.iter();
    files.for_each(|&(id, source)| {
        let result = lexer::lex_file(buckets, &mut tokens, env, id, source);
        match result {
            Err(err) => {
                errors.push(err);
            }
            Ok(_) => {}
        }
    });

    if errors.len() != 0 {
        return Err(errors);
    }

    while let Some(n) = buckets.next() {
        buckets = n;
    }
    buckets = buckets.force_next();

    let iter = files_list.into_iter().filter_map(|(file, _)| {
        while let Some(n) = buckets.next() {
            buckets = n;
        }

        match parser::parse_tokens(buckets, &tokens, &mut asts, file) {
            Ok(x) => return Some(x),
            Err(err) => {
                errors.push(err);
                return None;
            }
        }
    });
    let asts: Vec<ast::ASTProgram> = iter.collect();

    while let Some(n) = buckets.next() {
        buckets = n;
    }
    buckets = buckets.force_next();

    let mut assembler = assembler::Assembler::new();
    asts.into_iter().for_each(|ast| {
        while let Some(n) = buckets.next() {
            buckets = n;
        }

        let tfuncs = match type_checker::check_file(buckets, ast) {
            Ok(x) => x,
            Err(err) => {
                errors.push(err);
                return;
            }
        };

        match assembler.add_file(tfuncs) {
            Ok(()) => {}
            Err(err) => {
                errors.push(err);
            }
        }
    });

    if errors.len() != 0 {
        return Err(errors);
    }

    let program = match assembler.assemble(&env) {
        Ok(x) => x,
        Err(err) => return Err(err.into()),
    };

    while let Some(b) = unsafe { buckets_begin.dealloc() } {
        buckets_begin = b;
    }

    Ok(program)
}

fn emit_err(errs: &Vec<Error>, files: &FileDb, writer: &mut impl WriteColor) {
    let config = codespan_reporting::term::Config::default();
    for err in errs {
        codespan_reporting::term::emit(writer, &config, files, &err.diagnostic())
            .expect("why did this fail?");
    }
}

fn run(program: interpreter::Program, runtime_io: impl RuntimeIO) -> i32 {
    let mut runtime = interpreter::Runtime::new(runtime_io);
    runtime.run_program(program)
}

fn compile_from_program_args() {
    let args: Vec<String> = std::env::args().collect();

    let writer = StandardStream::stderr(ColorChoice::Always);
    let runtime_io = DefaultIO::new();

    let mut files = FileDb::new();
    for arg in args.iter().skip(1) {
        files.add(&arg).unwrap();
    }
    mem::drop(args);

    let program = match compile(&mut files) {
        Ok(program) => program,
        Err(errs) => {
            let config = codespan_reporting::term::Config::default();
            for err in errs {
                codespan_reporting::term::emit(
                    &mut writer.lock(),
                    &config,
                    &files,
                    &err.diagnostic(),
                )
                .expect("why did this fail?");
            }
            return;
        }
    };

    mem::drop(files);
}

enum GlobalState {
    Uninit,
    Args(Vec<String>),
    Compiled(Program<'static>),
}

static GLOBALS: LazyStatic<Mutex<GlobalState>> = lazy_static!(globals, Mutex<GlobalState>, {
    Mutex::new(GlobalState::Uninit)
});

fn main() -> Result<(), net_io::WebServerError> {
    let server = net_io::WebServer {
        http_handler: respond_to_http_request,
        ws_handler: ws_respond,
    };

    server.serve()?;
    Ok(())
}

// returns true to keep the connection open
fn respond_to_http_request<'a>(
    http_header: HttpHeader,
    buffer2: &'a mut [u8],
) -> Result<net_io::HttpResponse<'a>, WebServerError> {
    const ROOT_HTML: &str = "<!doctype html><html></html>";
    match http_header.path {
        "/" => {
            return Ok(net_io::HttpResponse {
                status: 200,
                body: ROOT_HTML.as_bytes(),
            });
        }
        "/favicon.ico" => {
            return Ok(net_io::HttpResponse {
                status: 404,
                body: "".as_bytes(),
            });
        }
        _ => {
            return Ok(net_io::HttpResponse {
                status: 404,
                body: ROOT_HTML.as_bytes(),
            });
        }
    }
}

fn ws_respond<'a>(
    message_type: WebSocketReceiveMessageType,
    ws_buffer: &[u8],
    out_buffer: &'a mut [u8],
) -> Result<net_io::WSResponse<'a>, WebServerError> {
    match message_type {
        WebSocketReceiveMessageType::Text => {
            let message = match std::str::from_utf8(ws_buffer) {
                Ok(m) => m,
                Err(err) => {
                    let len = write_b!(out_buffer, "received invalid UTF-8 ({})", err)?;

                    return Ok(net_io::WSResponse::Response {
                        message_type: WebSocketSendMessageType::Text,
                        message: &out_buffer[..len],
                    });
                }
            };

            let message = &mut out_buffer[..ws_buffer.len()];
            message.copy_from_slice(ws_buffer);

            return Ok(net_io::WSResponse::Response {
                message_type: WebSocketSendMessageType::Text,
                message,
            });
        }
        WebSocketReceiveMessageType::CloseCompleted => return Ok(net_io::WSResponse::Close),
        WebSocketReceiveMessageType::CloseMustReply => {
            let message = &mut out_buffer[..ws_buffer.len()];
            message.copy_from_slice(ws_buffer);

            return Ok(net_io::WSResponse::Response {
                message_type: WebSocketSendMessageType::CloseReply,
                message,
            });
        }
        WebSocketReceiveMessageType::Ping => {
            let message = &mut out_buffer[..ws_buffer.len()];
            message.copy_from_slice(ws_buffer);

            return Ok(net_io::WSResponse::Response {
                message_type: WebSocketSendMessageType::Pong,
                message,
            });
        }
        _ => return Ok(net_io::WSResponse::None),
    }
}
