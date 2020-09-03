use crate::ast::*;
use crate::buckets::*;
use crate::filedb::*;
use crate::interpreter::*;
use crate::runtime::*;
use crate::type_checker::*;
use crate::util::*;
use core::alloc;
use core::mem::{align_of, size_of};
use std::collections::{HashMap, HashSet};

pub struct ASMFunc<'a> {
    pub func_type: TCFuncType<'a>,
    pub func_header: Option<(u32, CodeLoc)>, // first u32 points into opcodes buffer
}

pub enum ASMAssignKind<'a> {
    StackLocal { var: i16 },
    Ptr(&'a TCExpr<'a>),
}

pub struct ASMAssign<'a> {
    kind: ASMAssignKind<'a>,
    assign_type: TCType,
    offset: u32,
    bytes: u32,
}

lazy_static! {
    pub static ref LIB_FUNCS: HashSet<u32> = {
        let mut m = HashSet::new();
        m.insert(INITIAL_SYMBOLS.translate["printf"]);
        m.insert(INITIAL_SYMBOLS.translate["exit"]);
        m
    };
}

pub struct Assembler<'a> {
    pub opcodes: Vec<TaggedOpcode>,
    pub func_types: HashMap<u32, u32>,
    pub data: VarBuffer,
    pub functions: HashMap<u32, ASMFunc<'a>>, // keys are identifier symbols
}

impl<'a> Assembler<'a> {
    pub fn new() -> Self {
        Self {
            opcodes: Vec::new(),
            data: VarBuffer::new(),
            functions: HashMap::new(),
            func_types: HashMap::new(),
        }
    }

    pub fn add_file(&mut self, types: TypedFuncs<'a>) -> Result<(), Error> {
        // TODO Add types here

        // Add function return sizes
        for (ident, TCFunc { func_type, .. }) in types.functions.iter() {
            self.func_types.insert(*ident, func_type.return_type.size());
        }

        // Add functions
        for (ident, func) in types.functions.into_iter() {
            self.add_function(ident, func)?;
        }

        return Ok(());
    }

    pub fn add_function(&mut self, ident: u32, func: TCFunc<'a>) -> Result<(), Error> {
        let (func_type, func_header) = match self.functions.get(&ident) {
            Some(asm_func) => {
                if asm_func.func_type != func.func_type {
                    let error = func_decl_mismatch(asm_func.func_type.loc, func.func_type.loc);
                    return Err(error);
                }
                (asm_func.func_type, asm_func.func_header)
            }
            None => (func.func_type, None),
        };

        let mut asm_func = ASMFunc {
            func_type,
            func_header: None,
        };

        let defn = match func.defn {
            Some(defn) => defn,
            None => {
                self.functions.insert(ident, asm_func);
                return Ok(());
            }
        };

        if let Some((func_header, defn_loc)) = func_header {
            return Err(func_redef(defn_loc, defn.loc));
        }

        asm_func.func_header = Some((self.opcodes.len() as u32, defn.loc));
        let param_count = func_type.params.len() as u32;

        self.opcodes.push(TaggedOpcode {
            op: Opcode::Func(ident),
            loc: defn.loc,
        });

        for stmt in defn.stmts {
            let mut ops = self.translate_statement(&func_type, param_count, stmt);
            self.opcodes.append(&mut ops);
        }

        self.opcodes.push(TaggedOpcode {
            op: Opcode::Ret,
            loc: defn.loc,
        });

        self.functions.insert(ident, asm_func);
        return Ok(());
    }

    pub fn translate_statement(
        &mut self,
        func_type: &TCFuncType,
        param_count: u32,
        stmt: &TCStmt,
    ) -> Vec<TaggedOpcode> {
        let mut ops = Vec::new();
        let mut tagged = TaggedOpcode {
            op: Opcode::StackDealloc,
            loc: stmt.loc,
        };

        match &stmt.kind {
            TCStmtKind::RetVal(expr) => {
                ops.append(&mut self.translate_expr(expr));

                let ret_idx = (param_count as i16 * -1) - 1;
                tagged.op = Opcode::SetLocal {
                    var: ret_idx,
                    offset: 0,
                    bytes: expr.expr_type.size(),
                };
                ops.push(tagged);
                tagged.op = Opcode::Ret;
                ops.push(tagged);
            }
            TCStmtKind::Ret => {
                tagged.op = Opcode::Ret;
                ops.push(tagged);
            }

            TCStmtKind::Expr(expr) => {
                ops.append(&mut self.translate_expr(expr));
                tagged.op = Opcode::Pop {
                    bytes: expr.expr_type.size(),
                };
                ops.push(tagged);
            }

            TCStmtKind::Decl { init } => {
                let bytes = init.expr_type.size();
                tagged.op = Opcode::StackAlloc(bytes);
                ops.push(tagged);
                ops.append(&mut self.translate_expr(init));
                tagged.op = Opcode::PopIntoTopVar { bytes, offset: 0 };
                ops.push(tagged);
            }
        }

        return ops;
    }

    pub fn translate_expr(&mut self, expr: &TCExpr) -> Vec<TaggedOpcode> {
        let mut ops = Vec::new();
        let mut tagged = TaggedOpcode {
            op: Opcode::StackDealloc,
            loc: expr.loc,
        };

        match &expr.kind {
            TCExprKind::Uninit => {
                tagged.op = Opcode::PushUndef {
                    bytes: expr.expr_type.size(),
                };
                ops.push(tagged);
            }
            TCExprKind::IntLiteral(val) => {
                tagged.op = Opcode::MakeTempInt32(*val);
                ops.push(tagged);
            }
            TCExprKind::StringLiteral(val) => {
                let var = self.data.add_var(val.len() as u32 + 1); // TODO overflow here
                let slice = self.data.get_full_var_range_mut(var);
                let end = slice.len() - 1;
                slice[..end].copy_from_slice(val.as_bytes());
                slice[end] = 0;
                tagged.op = Opcode::MakeTempBinaryPtr { var, offset: 0 };
                ops.push(tagged);
            }
            TCExprKind::LocalIdent { var_offset } => {
                tagged.op = Opcode::GetLocal {
                    var: *var_offset,
                    offset: 0,
                    bytes: expr.expr_type.size(),
                };
                ops.push(tagged);
            }

            TCExprKind::AddI32(l, r) => {
                ops.append(&mut self.translate_expr(l));
                ops.append(&mut self.translate_expr(r));
                tagged.op = Opcode::AddU32;
                ops.push(tagged);
            }
            TCExprKind::AddU64(l, r) => {
                ops.append(&mut self.translate_expr(l));
                ops.append(&mut self.translate_expr(r));
                tagged.op = Opcode::AddU64;

                ops.push(tagged);
            }
            TCExprKind::SubI32(l, r) => {
                ops.append(&mut self.translate_expr(l));
                ops.append(&mut self.translate_expr(r));
                tagged.op = Opcode::SubI32;
                ops.push(tagged);
            }

            TCExprKind::SConv8To32(expr) => {
                ops.append(&mut self.translate_expr(expr));
                tagged.op = Opcode::SExtend8To32;
                ops.push(tagged);
            }
            TCExprKind::SConv32To64(expr) => {
                ops.append(&mut self.translate_expr(expr));
                tagged.op = Opcode::SExtend32To64;
                ops.push(tagged);
            }

            TCExprKind::ZConv8To32(expr) => {
                ops.append(&mut self.translate_expr(expr));
                tagged.op = Opcode::ZExtend8To32;
                ops.push(tagged);
            }
            TCExprKind::ZConv32To64(expr) => {
                ops.append(&mut self.translate_expr(expr));
                tagged.op = Opcode::ZExtend32To64;
                ops.push(tagged);
            }

            TCExprKind::Assign { target, value } => {
                ops.append(&mut self.translate_expr(value));
                ops.append(&mut self.translate_assign(target));
            }

            TCExprKind::Member { base, offset } => {
                let base_bytes = base.expr_type.size();
                ops.append(&mut self.translate_expr(base));
                let want_bytes = expr.expr_type.size();
                let top_bytes = base_bytes - want_bytes - offset;
                tagged.op = Opcode::Pop { bytes: top_bytes };
                ops.push(tagged);
                tagged.op = Opcode::PopKeep {
                    drop: *offset,
                    keep: want_bytes,
                };
                ops.push(tagged);
            }
            TCExprKind::PtrMember { base, offset } => {
                let bytes = expr.expr_type.size();
                ops.append(&mut self.translate_expr(base));
                tagged.op = Opcode::Get {
                    offset: *offset,
                    bytes,
                };
                ops.push(tagged);
            }

            TCExprKind::Deref(ptr) => {
                ops.append(&mut self.translate_expr(ptr));
                tagged.op = Opcode::Get {
                    offset: 0,
                    bytes: expr.expr_type.size(),
                };
                ops.push(tagged);
            }
            TCExprKind::Ref(lvalue) => match lvalue.kind {
                TCAssignKind::LocalIdent { var_offset } => {
                    tagged.op = Opcode::MakeTempLocalStackPtr {
                        var: var_offset,
                        offset: 0,
                    };
                    ops.push(tagged);
                }
                TCAssignKind::Ptr(expr) => {
                    ops.append(&mut self.translate_expr(expr));
                }
            },

            TCExprKind::Call {
                func,
                params,
                varargs,
            } => {
                let rtype_size = *self.func_types.get(&func).unwrap();
                tagged.op = Opcode::StackAlloc(rtype_size);
                ops.push(tagged);

                for param in *params {
                    let bytes = param.expr_type.size();
                    tagged.op = Opcode::StackAlloc(bytes);
                    ops.push(tagged);
                    ops.append(&mut self.translate_expr(param));
                    tagged.op = Opcode::PopIntoTopVar { offset: 0, bytes };
                    ops.push(tagged);
                }

                if *varargs {
                    tagged.op = Opcode::StackAlloc(4);
                    ops.push(tagged);
                    tagged.op = Opcode::MakeTempInt32(params.len() as i32); // check overflow here
                    ops.push(tagged);
                    tagged.op = Opcode::PopIntoTopVar {
                        offset: 0,
                        bytes: 4,
                    };
                    ops.push(tagged);
                }

                tagged.op = Opcode::Call(*func);
                ops.push(tagged);

                tagged.op = Opcode::StackDealloc;
                for param in *params {
                    ops.push(tagged);
                }

                if *varargs {
                    ops.push(tagged);
                }

                if rtype_size == 0 {
                    tagged.op = Opcode::StackDealloc;
                    ops.push(tagged);
                } else {
                    tagged.op = Opcode::StackAddToTemp;
                    ops.push(tagged);
                }
            }
        }

        return ops;
    }

    pub fn translate_assign(&mut self, assign: &TCAssignTarget) -> Vec<TaggedOpcode> {
        let mut ops = Vec::new();
        let mut tagged = TaggedOpcode {
            op: Opcode::StackDealloc,
            loc: assign.target_loc,
        };

        match assign.kind {
            TCAssignKind::Ptr(expr) => {
                let bytes = assign.target_type.size();
                tagged.op = Opcode::PushDup { bytes };
                ops.push(tagged);
                ops.append(&mut self.translate_expr(expr));
                tagged.op = Opcode::Set { offset: 0, bytes };
                ops.push(tagged);
            }
            TCAssignKind::LocalIdent { var_offset } => {
                let (bytes, offset) = (assign.target_type.size(), assign.offset);
                tagged.op = Opcode::PushDup { bytes };
                ops.push(tagged);
                tagged.op = Opcode::SetLocal {
                    var: var_offset,
                    offset,
                    bytes,
                };
                ops.push(tagged);
            }
        }

        return ops;
    }

    pub fn translate_lvalue<'b>(&self, assign: &TCAssignTarget<'b>) -> ASMAssign<'b> {
        match &assign.kind {
            TCAssignKind::LocalIdent { var_offset } => {
                return ASMAssign {
                    kind: ASMAssignKind::StackLocal { var: *var_offset },
                    offset: 0,
                    bytes: assign.target_type.size(),
                    assign_type: assign.target_type,
                };
            }
            TCAssignKind::Ptr(expr) => {
                return ASMAssign {
                    kind: ASMAssignKind::Ptr(expr),
                    offset: 0,
                    bytes: assign.target_type.size(),
                    assign_type: assign.target_type,
                };
            }
        }
    }

    pub fn assemble<'b>(mut self, env: &FileDb) -> Result<Program<'b>, Error> {
        let no_main = || error!("missing main function definition");
        let main_func = self.functions.get(&INITIAL_SYMBOLS.translate["main"]).ok_or_else(no_main)?;
        let (main_idx, _main_loc) = main_func.func_header.ok_or_else(no_main)?;

        for op in self.opcodes.iter_mut() {
            match &mut op.op {
                Opcode::Call(addr) => {
                    let function = self.functions.get(addr).unwrap();
                    if let Some(func_header) = function.func_header {
                        let (fptr, loc) = func_header;
                        *addr = fptr;
                    } else if LIB_FUNCS.contains(addr) {
                        op.op = Opcode::LibCall(*addr);
                    } else {
                        let func_loc = function.func_type.loc;
                        return Err(error!(
                            "couldn't find definition for function",
                            op.loc, "called here", func_loc, "declared here"
                        ));
                    }
                }
                _ => {}
            }
        }

        let file_size = align_usize(env.size(), align_of::<TaggedOpcode>());
        let opcode_size = size_of::<TaggedOpcode>();
        let opcodes_size = self.opcodes.len() * opcode_size;
        let data_size = align_usize(self.data.data.len(), align_of::<Var>());
        let vars_size = self.data.vars.len() * size_of::<Var>();

        let total_size = file_size + opcodes_size + data_size + vars_size + 8;
        let buckets = BucketList::with_capacity(0);
        let layout = alloc::Layout::from_size_align(total_size, 8).expect("why did this fail?");
        let mut frame = buckets.alloc_frame(layout);

        let files = FileDbRef::new_from_frame(&mut frame, env);
        let ops = frame.add_array(self.opcodes);
        let data = self.data.write_to_ref(frame);

        let program = Program {
            files,
            data,
            ops,
            main_idx,
        };

        return Ok(program);
    }
}
