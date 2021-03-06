use crate::ast::*;
use crate::buckets::*;
use crate::filedb::*;
use crate::runtime::*;
use crate::util::*;
use core::fmt;
use serde::Serialize;
use std::collections::HashMap;
use std::io::Write;

macro_rules! error {
    ($arg1:tt,$($arg:tt)*) => {
        IError::new($arg1, format!($($arg)*))
    };
}

macro_rules! err {
    ($arg1:tt,$($arg:tt)*) => {
        Err(IError::new($arg1, format!($($arg)*)))
    };
}

pub fn render_err(error: &IError, stack_trace: &Vec<CallFrame>, program: &Program) -> String {
    use codespan_reporting::diagnostic::*;
    use codespan_reporting::term::*;

    let mut out = StringWriter::new();
    let config = Config {
        display_style: DisplayStyle::Rich,
        tab_width: 4,
        styles: Styles::default(),
        chars: Chars::default(),
        start_context_lines: 3,
        end_context_lines: 1,
    };

    write!(out, "{}: {}\n", error.short_name, error.message).unwrap();

    for frame in stack_trace.iter().skip(1) {
        let diagnostic = Diagnostic::new(Severity::Void)
            .with_labels(vec![Label::primary(frame.loc.file, frame.loc)]);
        codespan_reporting::term::emit(&mut out, &config, &program.files, &diagnostic).unwrap();
    }

    return out.to_string();
}

/// Exit the program with an error code
pub const ECALL_EXIT: u32 = 0;

/// Get the number of arguments in the program.
pub const ECALL_ARGC: u32 = 1;

/// Get zero-indexed command line argument. Takes in a single int as a parameter,
/// and pushes a pointer to the string on the heap as the result.
pub const ECALL_ARGV: u32 = 2;

/// No symbol associated with this stack/binary var
pub const META_NO_SYMBOL: u32 = u32::MAX;

/// - GetLocal gets a value from the stack at a given stack and variable offset
/// - SetLocal sets a value on the stack at a given stack and variable offset to the value at the top
///   of the stack
/// - Set and Get are equivalent of GetLocal and SetLocal, but the location they access is
///   determined by popping the top of the stack first
/// - PopKeep pops keep-many bytes off the stack, then pops drop-many bytes off the stack and
///   repushes the first set of popped bytes back onto  the stack
/// - CompLt compares pops t, the top of the stack, and compares it to n, the next item on the stack.
///   it pushes the byte 1 onto the stack if n < t, and the byte 0 onto the stack if n >= t.
/// - CompLeq compares pops t, the top of the stack, and compares it to n, the next item on the stack.
///   it pushes the byte 1 onto the stack if n <= t, and the byte 0 onto the stack if n > t.
/// - CompEq compares pops t, the top of the stack, and compares it to n, the next item on the stack.
///   it pushes the byte 1 onto the stack if n == t, and the byte 0 onto the stack if n != t.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(tag = "code", content = "data")]
pub enum Opcode {
    Func(u32), // Function header used for callstack manipulation

    StackAlloc { bytes: u32, symbol: u32 }, // Allocates space on the stack
    StackAllocDyn { symbol: u32 },          // Allocates space on the stack based on a u32 pop
    StackDealloc,                           // Pops a variable off of the stack
    StackAddToTemp, // Pops a variable off the stack, adding it to the temporary storage below

    MakeTempI8(i8),
    MakeTempI32(i32),
    MakeTempI64(i64),
    MakeTempU64(u64),
    MakeTempF64(f64),
    MakeTempBinaryPtr { var: u32, offset: u32 },
    MakeTempLocalStackPtr { var: i16, offset: u32 },

    Pop { bytes: u32 },
    PopKeep { keep: u32, drop: u32 },
    PushUndef { bytes: u32 },       // Push undefined bytes onto the stack
    PushDup { bytes: u32 },         // Push bytes duplicated from the top of the stack
    Swap { top: u32, bottom: u32 }, // Swap some number of top bytes with some number of bytes below
    PopIntoTopVar { offset: u32, bytes: u32 },

    SExtend8To16,
    SExtend8To32,
    SExtend8To64,
    SExtend16To32,
    SExtend16To64,
    SExtend32To64,

    ZExtend8To16,
    ZExtend8To32,
    ZExtend8To64,
    ZExtend16To32,
    ZExtend16To64,
    ZExtend32To64,

    GetLocal { var: i16, offset: u32, bytes: u32 },
    SetLocal { var: i16, offset: u32, bytes: u32 },

    Get { offset: u32, bytes: u32 },
    Set { offset: u32, bytes: u32 },

    AddU32,
    AddU64,

    SubI32,
    SubI64,
    SubU64,

    MulI32,

    DivI32,
    DivI64,
    DivU64,

    CompLtI32,
    CompLtU64,
    CompLeqI32,
    CompLeqU64,

    CompEq32,
    CompNeq32,
    CompEq64,

    MulI64,
    MulU64,
    ModI64,

    Jump(u32),

    JumpIfZero8(u32),
    JumpIfZero16(u32),
    JumpIfZero32(u32),
    JumpIfZero64(u32),

    JumpIfNotZero8(u32),
    JumpIfNotZero16(u32),
    JumpIfNotZero32(u32),
    JumpIfNotZero64(u32),

    Ret, // Returns to caller

    Call(u32),
    LibCall(u32),
    Ecall(u32),
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct TaggedOpcode {
    pub op: Opcode,
    pub loc: CodeLoc,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct RuntimeVar {
    pub decl_type: TCType,
    pub symbol: u32,
    pub loc: CodeLoc,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct RuntimeStruct<'a> {
    pub members: Option<&'a [TCStructMember]>,
    pub loc: CodeLoc,
    pub sa: SizeAlign,
}

#[derive(Clone, Copy, Serialize)]
pub struct Program<'a> {
    #[serde(skip)]
    pub buckets: BucketListRef<'a>,
    pub files: FileDbRef<'a>,
    pub types: HashRef<'a, u32, RuntimeStruct<'a>>,
    pub symbols: &'a [RuntimeVar],
    pub data: VarBufferRef<'a>,
    pub ops: &'a [TaggedOpcode],
}

impl<'a> fmt::Debug for Program<'a> {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        fmt.debug_struct("Program")
            .field("files", &self.files)
            .field("types", &self.types)
            .field("symbols", &self.symbols)
            .field("data", &self.data)
            .field("ops", &self.ops)
            .finish()
    }
}

type LibFunc = for<'a> fn(&'a mut Runtime) -> Result<Option<i32>, IError>;

#[derive(Debug, Serialize)]
pub struct RuntimeDiagnostic {
    pub callstack: u32,
    pub fp: u16,
    pub pc: u32,
    pub loc: CodeLoc,
}

pub struct Runtime {
    pub memory: Memory,
    pub args: StringArray,
    pub lib_funcs: HashMap<u32, LibFunc>,
    pub program: Program<'static>,
}

impl Runtime {
    pub fn new(program: Program<'static>, args: StringArray) -> Self {
        let mut lib_funcs: HashMap<u32, LibFunc> = HashMap::new();

        lib_funcs.insert(INIT_SYMS.translate["printf"], printf);
        lib_funcs.insert(INIT_SYMS.translate["exit"], exit);
        lib_funcs.insert(INIT_SYMS.translate["malloc"], malloc);
        lib_funcs.insert(INIT_SYMS.translate["realloc"], realloc);
        lib_funcs.insert(INIT_SYMS.translate["memcpy"], memcpy);

        let memory = Memory::new_with_binary(program.data);
        let s = Self {
            args,
            memory,
            program,
            lib_funcs,
        };
        return s;
    }

    pub fn diagnostic(&self) -> RuntimeDiagnostic {
        RuntimeDiagnostic {
            callstack: self.memory.callstack.len() as u32, // TODO handle overflow
            fp: self.memory.fp,
            pc: self.memory.pc,
            loc: self.program.ops[self.memory.pc as usize].loc,
        }
    }

    pub fn run(&mut self, mut io: impl Write) -> Result<i32, IError> {
        loop {
            let ret = self.run_op();
            for event in self.memory.events() {
                let string = event.to_string();
                write!(io, "{}", string)?;
            }

            if let Some(exit) = ret? {
                return Ok(exit);
            }
        }
    }

    pub fn run_op_count(&mut self, mut count: u32) -> Result<Option<i32>, IError> {
        let callstack_len = self.memory.callstack.len();
        while count > 0 {
            if let Some(exit) = self.run_op()? {
                return Ok(Some(exit));
            }
            count -= 1;
        }

        return Ok(None);
    }

    pub fn run_count_or_until(
        &mut self,
        mut count: u32,
        pc: u32,
        stack_size: u16,
    ) -> Result<Option<i32>, IError> {
        let stack_size = stack_size as usize;
        while stack_size <= self.memory.callstack.len() && count > 0 && self.memory.pc != pc {
            if let Some(exit) = self.run_op()? {
                return Ok(Some(exit));
            }

            count -= 1;
        }

        return Ok(None);
    }

    pub fn run_op(&mut self) -> Result<Option<i32>, IError> {
        match self.run_op_internal() {
            Ok(opt) => return Ok(opt),
            Err(err) => {
                self.memory
                    .push_callstack(self.program.ops[self.memory.pc as usize].loc);
                return Err(err);
            }
        }
    }

    #[inline]
    pub fn run_op_internal(&mut self) -> Result<Option<i32>, IError> {
        let op = self.program.ops[self.memory.pc as usize];
        // write!(self.io.log(), "op: {:?}\n", op.op)
        // .map_err(|err| error!("WriteFailed", "failed to write to logs ({})", err))?;

        let opcode = op.op;
        match opcode {
            Opcode::Func(_) => {}

            Opcode::StackAlloc { bytes, symbol } => {
                self.memory.add_stack_var(bytes, symbol);
            }
            Opcode::StackAllocDyn { symbol } => {
                let space = u32::from_be(self.memory.pop_stack()?);
                self.memory.add_stack_var(space, symbol);
            }
            Opcode::StackDealloc => {
                self.memory.pop_stack_var()?;
            }
            Opcode::StackAddToTemp => {
                self.memory.pop_stack_var_onto_stack()?;
            }

            Opcode::MakeTempI8(value) => self.memory.push_stack(value),
            Opcode::MakeTempI32(value) => self.memory.push_stack(value.to_be()),
            Opcode::MakeTempI64(value) => self.memory.push_stack(value.to_be()),
            Opcode::MakeTempU64(value) => self.memory.push_stack(value.to_be()),
            Opcode::MakeTempF64(value) => self.memory.push_stack(value),
            Opcode::MakeTempBinaryPtr { var, offset } => {
                let ptr = VarPointer::new_binary(var, offset);
                self.memory.push_stack(ptr);
            }
            Opcode::MakeTempLocalStackPtr { var, offset } => {
                let ptr = VarPointer::new_stack(self.memory.fp_offset(var), offset);
                self.memory.push_stack(ptr);
            }

            Opcode::Pop { bytes } => self.memory.pop_bytes(bytes)?,
            Opcode::PopKeep { keep, drop } => self.memory.pop_keep_bytes(keep, drop)?,
            Opcode::PushUndef { bytes } => {
                self.memory.add_stack_var(bytes, META_NO_SYMBOL);
                self.memory.pop_stack_var_onto_stack().unwrap();
            }
            Opcode::PushDup { bytes } => {
                self.memory.dup_top_stack_bytes(bytes)?;
            }
            Opcode::Swap { top, bottom } => {
                self.memory.dup_top_stack_bytes(top + bottom)?;
                self.memory.pop_bytes(top)?;
                self.memory.pop_keep_bytes(top + bottom, bottom)?;
            }
            Opcode::PopIntoTopVar { offset, bytes } => {
                let ptr = VarPointer::new_stack(self.memory.stack_length(), offset);
                self.memory.pop_stack_bytes_into(ptr, bytes)?;
            }

            Opcode::SExtend8To16 => {
                let val = self.memory.pop_stack::<i8>()?;
                self.memory.push_stack((val as i16).to_be());
            }
            Opcode::SExtend8To32 => {
                let val = self.memory.pop_stack::<i8>()?;
                self.memory.push_stack((val as i32).to_be());
            }
            Opcode::SExtend8To64 => {
                let val = self.memory.pop_stack::<i8>()?;
                self.memory.push_stack((val as i64).to_be());
            }
            Opcode::SExtend16To32 => {
                let val = i16::from_be(self.memory.pop_stack()?);
                self.memory.push_stack((val as i32).to_be());
            }
            Opcode::SExtend16To64 => {
                let val = i16::from_be(self.memory.pop_stack()?);
                self.memory.push_stack((val as i64).to_be());
            }
            Opcode::SExtend32To64 => {
                let val = i32::from_be(self.memory.pop_stack()?);
                self.memory.push_stack((val as i64).to_be());
            }

            Opcode::ZExtend8To16 => {
                let val = self.memory.pop_stack::<u8>()?;
                self.memory.push_stack((val as u16).to_be());
            }
            Opcode::ZExtend8To32 => {
                let val = self.memory.pop_stack::<u8>()?;
                self.memory.push_stack((val as u32).to_be());
            }
            Opcode::ZExtend8To64 => {
                let val = self.memory.pop_stack::<u8>()?;
                self.memory.push_stack((val as u64).to_be());
            }
            Opcode::ZExtend16To32 => {
                let val = u16::from_be(self.memory.pop_stack()?);
                self.memory.push_stack((val as u32).to_be());
            }
            Opcode::ZExtend16To64 => {
                let val = u16::from_be(self.memory.pop_stack()?);
                self.memory.push_stack((val as u64).to_be());
            }
            Opcode::ZExtend32To64 => {
                let val = u32::from_be(self.memory.pop_stack()?);
                self.memory.push_stack((val as u64).to_be());
            }

            Opcode::GetLocal { var, offset, bytes } => {
                let ptr = VarPointer::new_stack(self.memory.fp_offset(var), offset);
                self.memory.push_stack_bytes_from(ptr, bytes)?;
            }
            Opcode::SetLocal { var, offset, bytes } => {
                let ptr = VarPointer::new_stack(self.memory.fp_offset(var), offset);
                self.memory.pop_stack_bytes_into(ptr, bytes)?;
            }

            Opcode::Get { offset, bytes } => {
                let ptr: VarPointer = self.memory.pop_stack()?;
                // TODO check for overflow
                let ptr = ptr.with_offset(ptr.offset().wrapping_add(offset as u32));
                self.memory.push_stack_bytes_from(ptr, bytes)?;
            }
            Opcode::Set { offset, bytes } => {
                let ptr: VarPointer = self.memory.pop_stack()?;
                // TODO check for overflow
                let ptr = ptr.with_offset(ptr.offset().wrapping_add(offset as u32));
                self.memory.pop_stack_bytes_into(ptr, bytes)?;
            }

            Opcode::AddU32 => {
                let word2 = u32::from_be(self.memory.pop_stack()?);
                let word1 = u32::from_be(self.memory.pop_stack()?);
                self.memory.push_stack(word1.wrapping_add(word2).to_be());
            }
            Opcode::SubI32 => {
                let word2 = i32::from_be(self.memory.pop_stack()?);
                let word1 = i32::from_be(self.memory.pop_stack()?);
                self.memory.push_stack(word1.wrapping_sub(word2).to_be());
            }
            Opcode::MulI32 => {
                let word2 = i32::from_be(self.memory.pop_stack()?);
                let word1 = i32::from_be(self.memory.pop_stack()?);
                self.memory.push_stack(word1.wrapping_mul(word2).to_be());
            }
            Opcode::DivI32 => {
                let word2 = i32::from_be(self.memory.pop_stack()?);
                let word1 = i32::from_be(self.memory.pop_stack()?);
                self.memory.push_stack(word1.wrapping_div(word2).to_be());
            }
            Opcode::DivU64 => {
                let word2 = u64::from_be(self.memory.pop_stack()?);
                let word1 = u64::from_be(self.memory.pop_stack()?);
                self.memory.push_stack(word1.wrapping_div(word2).to_be());
            }

            Opcode::CompLeqI32 => {
                let word2 = i32::from_be(self.memory.pop_stack()?);
                let word1 = i32::from_be(self.memory.pop_stack()?);
                self.memory.push_stack((word1 <= word2) as u8);
            }
            Opcode::CompLtI32 => {
                let word2 = i32::from_be(self.memory.pop_stack()?);
                let word1 = i32::from_be(self.memory.pop_stack()?);
                self.memory.push_stack((word1 < word2) as u8);
            }

            Opcode::CompLeqU64 => {
                let word2 = u64::from_be(self.memory.pop_stack()?);
                let word1 = u64::from_be(self.memory.pop_stack()?);
                self.memory.push_stack((word1 <= word2) as u8);
            }
            Opcode::CompLtU64 => {
                let word2 = u64::from_be(self.memory.pop_stack()?);
                let word1 = u64::from_be(self.memory.pop_stack()?);
                self.memory.push_stack((word1 < word2) as u8);
            }

            Opcode::CompEq32 => {
                let word2 = i32::from_be(self.memory.pop_stack()?);
                let word1 = i32::from_be(self.memory.pop_stack()?);
                self.memory.push_stack((word1 == word2) as u8);
            }
            Opcode::CompNeq32 => {
                let word2 = i32::from_be(self.memory.pop_stack()?);
                let word1 = i32::from_be(self.memory.pop_stack()?);
                self.memory.push_stack((word1 != word2) as u8);
            }
            Opcode::CompEq64 => {
                let word2 = u64::from_be(self.memory.pop_stack()?);
                let word1 = u64::from_be(self.memory.pop_stack()?);
                self.memory.push_stack((word1 == word2) as u8);
            }

            Opcode::AddU64 => {
                let word2 = u64::from_be(self.memory.pop_stack()?);
                let word1 = u64::from_be(self.memory.pop_stack()?);

                self.memory.push_stack(word1.wrapping_add(word2).to_be());
            }
            Opcode::SubI64 => {
                let word2 = i64::from_be(self.memory.pop_stack()?);
                let word1 = i64::from_be(self.memory.pop_stack()?);
                self.memory.push_stack(word1.wrapping_sub(word2).to_be());
            }
            Opcode::SubU64 => {
                let word2 = u64::from_be(self.memory.pop_stack()?);
                let word1 = u64::from_be(self.memory.pop_stack()?);
                self.memory.push_stack(word1.wrapping_sub(word2).to_be());
            }
            Opcode::MulI64 => {
                let word2 = i64::from_be(self.memory.pop_stack()?);
                let word1 = i64::from_be(self.memory.pop_stack()?);
                self.memory.push_stack(word1.wrapping_mul(word2).to_be());
            }
            Opcode::MulU64 => {
                let word2 = u64::from_be(self.memory.pop_stack()?);
                let word1 = u64::from_be(self.memory.pop_stack()?);
                self.memory.push_stack(word1.wrapping_mul(word2).to_be());
            }
            Opcode::DivI64 => {
                let word2 = i64::from_be(self.memory.pop_stack()?);
                let word1 = i64::from_be(self.memory.pop_stack()?);
                self.memory.push_stack(word1.wrapping_div(word2).to_be());
            }
            Opcode::ModI64 => {
                let word2 = u64::from_be(self.memory.pop_stack()?);
                let word1 = u64::from_be(self.memory.pop_stack()?);
                self.memory.push_stack((word1 % word2).to_be());
            }

            Opcode::Jump(target) => {
                self.memory.jump(target);
                return Ok(None);
            }

            Opcode::JumpIfZero8(target) => {
                let value: u8 = self.memory.pop_stack()?;
                if value == 0 {
                    self.memory.jump(target);
                    return Ok(None);
                }
            }
            Opcode::JumpIfZero16(target) => {
                let value: u16 = self.memory.pop_stack()?;
                if value == 0 {
                    self.memory.jump(target);
                    return Ok(None);
                }
            }
            Opcode::JumpIfZero32(target) => {
                let value: u32 = self.memory.pop_stack()?;
                if value == 0 {
                    self.memory.jump(target);
                    return Ok(None);
                }
            }
            Opcode::JumpIfZero64(target) => {
                let value: u64 = self.memory.pop_stack()?;
                if value == 0 {
                    self.memory.jump(target);
                    return Ok(None);
                }
            }

            Opcode::JumpIfNotZero8(target) => {
                let value: u8 = self.memory.pop_stack()?;
                if value != 0 {
                    self.memory.jump(target);
                    return Ok(None);
                }
            }
            Opcode::JumpIfNotZero16(target) => {
                let value: u16 = self.memory.pop_stack()?;
                if value != 0 {
                    self.memory.jump(target);
                    return Ok(None);
                }
            }
            Opcode::JumpIfNotZero32(target) => {
                let value: u32 = self.memory.pop_stack()?;
                if value != 0 {
                    self.memory.jump(target);
                    return Ok(None);
                }
            }
            Opcode::JumpIfNotZero64(target) => {
                let value: u64 = self.memory.pop_stack()?;
                if value != 0 {
                    self.memory.jump(target);
                    return Ok(None);
                }
            }

            Opcode::Ret => {
                self.memory.ret();
                return Ok(None);
            }

            Opcode::Call(func) => {
                let func_name = match self.program.ops[func as usize].op {
                    Opcode::Func(name) => name,
                    op => panic!("found function header {:?} (this is an error in tci)", op),
                };
                self.memory.call(func + 1, func_name, op.loc);
                return Ok(None);
            }
            Opcode::LibCall(func_name) => {
                if let Some(lib_func) = self.lib_funcs.get(&func_name) {
                    lib_func(self)?;
                } else {
                    return Err(error!(
                        "InvalidLibraryFunction",
                        "library function symbol '{}' is invalid (this is a problem with tci)",
                        self.program.files.symbols[func_name as usize]
                    ));
                }
            }

            Opcode::Ecall(ECALL_EXIT) => {
                return Ok(Some(i32::from_be(self.memory.pop_stack()?)));
            }
            Opcode::Ecall(ECALL_ARGC) => {
                self.memory.push_stack((self.args.len() as u32).to_be());
            }
            Opcode::Ecall(ECALL_ARGV) => {
                let arg_idx: u32 = self.memory.pop_stack()?;
                let arg_idx = arg_idx as usize;
                if arg_idx >= self.args.len() {
                    return Err(error!(
                        "InvalidArgumentIndex",
                        "Argument index {} is invalid (this is a problem with tci)", arg_idx
                    ));
                }

                let arg = &self.args[arg_idx].as_bytes();
                let var_pointer = self.memory.add_heap_var(arg.len() as u32 + 1);
                let str_bytes = self.memory.get_var_slice_mut(var_pointer).unwrap();
                str_bytes[..arg.len()].copy_from_slice(arg);
                str_bytes[arg.len()] = 0;
                self.memory.push_stack(var_pointer);
            }
            Opcode::Ecall(call) => {
                return err!("InvalidEnviromentCall", "invalid ecall value of {}", call);
            }
        }

        self.memory.increment_pc();
        return Ok(None);
    }

    pub fn cstring_bytes(&self, ptr: VarPointer) -> Result<&[u8], IError> {
        let str_bytes = self.memory.get_var_slice(ptr)?;

        let mut idx = str_bytes.len();
        for (idx_, byte) in str_bytes.iter().enumerate() {
            if *byte == 0 {
                idx = idx_;
                break;
            }
        }

        if idx == str_bytes.len() {
            return err!("MissingNullTerminator", "string missing null terminator");
        }

        return Ok(&str_bytes[0..idx]);
    }
}

pub fn memcpy(sel: &mut Runtime) -> Result<Option<i32>, IError> {
    let stack_len = sel.memory.stack_length();
    let size_param_ptr = VarPointer::new_stack(stack_len, 0);
    let src_param_ptr = VarPointer::new_stack(stack_len - 1, 0);
    let dest_param_ptr = VarPointer::new_stack(stack_len - 2, 0);
    let ret_ptr = VarPointer::new_stack(stack_len - 3, 0);

    let size = u64::from_be(sel.memory.get_var(size_param_ptr)?);
    let mut src: VarPointer = sel.memory.get_var(src_param_ptr)?;
    let mut dest: VarPointer = sel.memory.get_var(dest_param_ptr)?;
    let to_ret = dest;

    for _ in 0..size {
        // PERFORMANCE this is so slow lmao
        let byte: u8 = sel.memory.get_var(src)?;
        sel.memory.set(dest, byte)?;
        src = src.add(1);
        dest = dest.add(1);
    }

    sel.memory.set(ret_ptr, to_ret)?;
    return Ok(None);
}

pub fn malloc(sel: &mut Runtime) -> Result<Option<i32>, IError> {
    let top_ptr = VarPointer::new_stack(sel.memory.stack_length(), 0);
    let ret_ptr = VarPointer::new_stack(sel.memory.stack_length() - 1, 0);
    let size = u64::from_be(sel.memory.get_var(top_ptr)?);
    let var_pointer = sel.memory.add_heap_var(size as u32); // TODO overflow
    sel.memory.set(ret_ptr, var_pointer)?;
    return Ok(None);
}

pub fn realloc(sel: &mut Runtime) -> Result<Option<i32>, IError> {
    let stack_len = sel.memory.stack_length();
    let size_ptr = VarPointer::new_stack(stack_len, 0);
    let to_free_ptr = VarPointer::new_stack(stack_len - 1, 0);
    let ret_ptr = VarPointer::new_stack(stack_len - 2, 0);

    let to_free: VarPointer = sel.memory.get_var(to_free_ptr)?;
    if !to_free.is_heap() {
        return Err(error!(
            "InvalidPointer",
            "called realloc on a pointer that was not from the heap"
        ));
    }

    if to_free.offset() != 0 {
        return Err(error!(
            "InvalidPointer",
            "called realloc on a pointer that was {} bytes off of its allocation",
            to_free.offset()
        ));
    }

    let size = u64::from_be(sel.memory.get_var(size_ptr)?);
    let new_alloc_ptr = sel.memory.add_heap_var(size as u32); // TODO overflow

    let (mut src, mut dest) = (to_free, new_alloc_ptr);
    for _ in 0..size {
        // PERFORMANCE this is so slow lmao
        let byte: u8 = sel.memory.get_var(src)?;
        sel.memory.set(dest, byte)?;
        src = src.add(1);
        dest = dest.add(1);
    }

    sel.memory.set(ret_ptr, new_alloc_ptr)?;
    return Ok(None);
}

pub fn free(sel: &mut Runtime) -> Result<Option<i32>, IError> {
    let top_ptr = VarPointer::new_stack(sel.memory.stack_length(), 0);
    let to_free: VarPointer = sel.memory.get_var(top_ptr)?;
    return Ok(None);
}

pub fn exit(sel: &mut Runtime) -> Result<Option<i32>, IError> {
    let top_ptr = VarPointer::new_stack(sel.memory.stack_length(), 0);
    let exit_code = i32::from_be(sel.memory.get_var(top_ptr)?);
    return Ok(Some(exit_code));
}

pub fn printf(sel: &mut Runtime) -> Result<Option<i32>, IError> {
    let top_ptr_offset = sel.memory.stack_length();
    let top_ptr = VarPointer::new_stack(top_ptr_offset, 0);
    let param_len = i32::from_be(sel.memory.get_var(top_ptr)?);

    let mut current_offset = top_ptr_offset - (param_len as u16);
    let return_offset = current_offset - 1;
    let format_ptr = VarPointer::new_stack(current_offset, 0); // TODO overflow
    current_offset += 1;

    let mut out = StringWriter::new();

    let result = printf_internal(sel, format_ptr, current_offset, &mut out);
    let out = out.into_string();
    let len = out.len() as i32; // TODO overflow
    write!(sel.memory.stdout(), "{}", out)?;
    result?;

    let return_ptr = VarPointer::new_stack(return_offset, 0); // TODO overflow
    sel.memory.set(return_ptr, len.to_be())?;

    return Ok(None);
}

#[allow(unused_assignments)] // TODO remove this when we make this fully standard compliant
pub fn printf_internal(
    sel: &mut Runtime,
    format_ptr: VarPointer,
    mut current_offset: u16,
    mut out: &mut StringWriter,
) -> Result<(), IError> {
    // OPTIMIZE This does an unnecessary linear scan
    let format_str = sel.cstring_bytes(sel.memory.get_var(format_ptr)?)?;
    let map_err = |err| error!("WriteFailed", "failed to write to stdout ({})", err);

    // CREDIT heavily inspired by https://github.com/mpaland/printf/blob/master/printf.c

    const FLAGS_ZEROPAD: u32 = 1;
    const FLAGS_LEFT: u32 = 2;
    const FLAGS_PLUS: u32 = 4;
    const FLAGS_SPACE: u32 = 8;
    const FLAGS_HASH: u32 = 16;
    const FLAGS_PRECISION: u32 = 32;
    const FLAGS_LONG: u32 = 64;
    const FLAGS_LONG_LONG: u32 = 128;

    let mut next_ptr = || {
        let var_ptr = VarPointer::new_stack(current_offset, 0);
        current_offset += 1;
        return var_ptr;
    };

    let parse_int = |begin: usize| {
        let mut idx = begin;
        if format_str[idx] >= b'0' && format_str[idx] <= b'9' {
            let mut collect = 0;
            loop {
                collect *= 10;
                collect += (format_str[idx] - b'0') as usize;
                idx += 1;

                if format_str[idx] < b'0' || format_str[idx] > b'9' {
                    break;
                }
            }

            return Some((collect, idx - begin));
        }

        return None;
    };

    let mut idx = 0;
    while idx < format_str.len() {
        let mut idx2 = idx;
        while idx2 < format_str.len() && format_str[idx2] != b'%' {
            idx2 += 1;
        }

        write_utf8_lossy(&mut out, &format_str[idx..idx2]).map_err(map_err)?;

        if idx2 == format_str.len() {
            break;
        }

        // format_str[idx2] == b'%'

        idx2 += 1;
        if idx2 == format_str.len() {
            return Err(error!(
                "InvalidFormatString",
                "format string ends with a single '%'; to print out a '%' use '%%'"
            ));
        }

        // format specifier?  %[flags][width][.precision][length]
        let mut flags = 0;
        let mut width = 0;
        let mut precision = 0;

        loop {
            match format_str[idx2] {
                b'0' => flags |= FLAGS_ZEROPAD,
                b'-' => flags |= FLAGS_LEFT,
                b'+' => flags |= FLAGS_PLUS,
                b' ' => flags |= FLAGS_SPACE,
                b'#' => flags |= FLAGS_HASH,
                _ => break,
            }
            idx += 1;
        }

        if let Some((w, diff)) = parse_int(idx2) {
            idx2 += diff;
            width = w;
        } else if format_str[idx2] == b'*' {
            let mut next = i32::from_be(sel.memory.get_var(next_ptr())?);
            if next < 0 {
                flags |= FLAGS_LEFT;
                next *= -1;
            }

            width = next as usize;
            idx2 += 1;
        }

        if format_str[idx2] == b'.' {
            flags |= FLAGS_PRECISION;
            idx2 += 1;

            if let Some((prec, diff)) = parse_int(idx2) {
                idx2 += diff;
                precision = prec;
            } else if format_str[idx2] == b'*' {
                let next = i32::from_be(sel.memory.get_var(next_ptr())?);
                precision = if next > 0 { next } else { 0 } as usize;
                idx2 += 1;
            }
        }

        match format_str[idx2] {
            b'l' => {
                flags |= FLAGS_LONG;
                idx2 += 1;
                if format_str[idx2] == b'l' {
                    flags |= FLAGS_LONG_LONG;
                    idx2 += 1;
                }
            }
            _ => {}
        }

        match format_str[idx2] {
            b'u' => {
                let base = 10;
                flags &= !FLAGS_HASH;
                if (flags & FLAGS_PRECISION) != 0 {
                    flags &= !FLAGS_ZEROPAD;
                }
                flags &= !(FLAGS_PLUS | FLAGS_SPACE);

                if (flags & FLAGS_LONG_LONG) != 0 {
                    let value = u64::from_be(sel.memory.get_var(next_ptr())?);
                    write!(&mut out, "{}", value).map_err(map_err)?;
                } else if (flags & FLAGS_LONG) != 0 {
                    let value = u64::from_be(sel.memory.get_var(next_ptr())?);
                    write!(&mut out, "{}", value).map_err(map_err)?;
                } else {
                    let value = u32::from_be(sel.memory.get_var(next_ptr())?);
                    write!(&mut out, "{}", value).map_err(map_err)?;
                }
            }
            b'i' | b'd' => {
                let base = 10;
                flags &= !FLAGS_HASH;
                if (flags & FLAGS_PRECISION) != 0 {
                    flags &= !FLAGS_ZEROPAD;
                }

                if (flags & FLAGS_LONG_LONG) != 0 {
                    let value = i64::from_be(sel.memory.get_var(next_ptr())?);
                    write!(&mut out, "{}", value).map_err(map_err)?;
                } else if (flags & FLAGS_LONG) != 0 {
                    let value = i64::from_be(sel.memory.get_var(next_ptr())?);
                    write!(&mut out, "{}", value).map_err(map_err)?;
                } else {
                    let value = i32::from_be(sel.memory.get_var(next_ptr())?);
                    write!(&mut out, "{}", value).map_err(map_err)?;
                }
            }
            b'c' => {
                let value: u8 = sel.memory.get_var(next_ptr())?;
                write!(&mut out, "{}", char::from(value)).map_err(map_err)?;
            }
            b'%' => {
                write_utf8_lossy(&mut out, &[b'%']).map_err(map_err)?;
            }
            b's' => {
                let char_ptr = sel.memory.get_var(next_ptr())?;

                write_utf8_lossy(&mut out, sel.cstring_bytes(char_ptr)?).map_err(map_err)?;
            }
            byte => {
                return Err(error!(
                    "InvalidFormatString",
                    "got byte '{}' after '%'",
                    char::from(byte)
                ))
            }
        }

        idx = idx2 + 1;
    }

    return Ok(());
}
