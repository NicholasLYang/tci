#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(incomplete_features)]

#[macro_use]
mod util;

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

use crate::filedb::FileDb;
use codespan_reporting::term::termcolor::{ColorChoice, StandardStream, WriteColor};
use core::mem;
use interpreter::Program;
use runtime::{DefaultIO, RuntimeIO};
use std::collections::HashSet;
use std::env;
use util::Error;

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
        // println!("Tokens: {:?}", result);
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

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let args: Vec<String> = env::args().collect();

    let writer = StandardStream::stderr(ColorChoice::Always);
    let runtime_io = DefaultIO::new();

    let mut flags = HashSet::new();
    let mut files = FileDb::new();

    for arg in args.iter().skip(1) {
        if arg.starts_with("-") {
            flags.insert(arg.clone());
            continue;
        }
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

            return Ok(());
        }
    };
    mem::drop(files);

    if !flags.contains("--debug") {
        let ret_code = run(program, runtime_io);
        eprintln!("TCI: return code was {}", ret_code);
        std::process::exit(ret_code as i32);
    }

    actix_web::HttpServer::new(|| actix_web::App::new())
        .bind("127.0.0.1:8080")?
        .run()
        .await
}
