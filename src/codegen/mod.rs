use std::collections::HashMap;

use inkwell::{
    AddressSpace, IntPredicate, OptimizationLevel,
    builder::{Builder, BuilderError},
    context::Context,
    module::{Linkage, Module},
    support::LLVMString,
    types::{FunctionType, IntType, PointerType, StructType},
    values::{BasicMetadataValueEnum, FunctionValue, GlobalValue, IntValue, PointerValue},
};
use miette::{Diagnostic, SourceSpan};
use thiserror::Error;

use crate::{ast, patternmatch, typecheck};

mod expr;
mod machine;
mod runtime;
mod target;
#[cfg(test)]
mod tests;

#[derive(Debug, Error, Diagnostic)]
pub enum CodegenError {
    #[error("LLVM builder error: {0}")]
    #[diagnostic(code(codegen::llvm))]
    Builder(#[from] BuilderError),
    #[error("unknown variable `{name}`")]
    UnknownVariable {
        name: String,
        #[label("not defined")]
        span: SourceSpan,
    },
    #[error("LLVM error: {0}")]
    #[diagnostic(code(codegen::llvm))]
    Llvm(String),
    #[error("linking failed: {0}")]
    #[diagnostic(code(codegen::link), help("linking uses $CC, or `clang`"))]
    Link(String),
    #[error("unknown type T_{id}")]
    #[diagnostic(code(codegen::unknown_type))]
    UnknownType { id: usize },
}

impl From<LLVMString> for CodegenError {
    fn from(e: LLVMString) -> Self {
        CodegenError::Llvm(e.to_string())
    }
}

type Result<T> = std::result::Result<T, CodegenError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResultKind {
    Unit,
    Bool,
    Char,
    Int,
    String,
    Function,
    Tuple(Vec<ResultKind>),
    Named(typecheck::TypeId, Vec<ResultKind>),
    /// no return value expected
    Never,
    /// always return the unboxed result as the exit code
    #[cfg(test)]
    ExitCode,
}

pub fn result_kind(tp: &typecheck::TypeExpr, type_args: &[ResultKind]) -> ResultKind {
    match tp {
        typecheck::TypeExpr::Unit => ResultKind::Unit,
        typecheck::TypeExpr::Bool => ResultKind::Bool,
        typecheck::TypeExpr::Char => ResultKind::Char,
        typecheck::TypeExpr::Int => ResultKind::Int,
        typecheck::TypeExpr::String => ResultKind::String,
        typecheck::TypeExpr::Named(id, args) => ResultKind::Named(
            *id,
            args.iter().map(|t| result_kind(t, type_args)).collect(),
        ),
        typecheck::TypeExpr::Product(elts) => {
            let kinds: Vec<ResultKind> = elts.iter().map(|t| result_kind(t, type_args)).collect();
            if kinds.contains(&ResultKind::Never) {
                ResultKind::Never
            } else {
                ResultKind::Tuple(kinds)
            }
        }
        typecheck::TypeExpr::Arrow(..) => ResultKind::Function,
        typecheck::TypeExpr::Var(index, _) => usize::try_from(*index)
            .ok()
            .and_then(|i| type_args.get(i))
            .cloned()
            .unwrap_or(ResultKind::Never),
        typecheck::TypeExpr::UnifVar(_) => ResultKind::Never,
    }
}

const TAIL_CC: u32 = 18;

const RUNTIME: &str = include_str!("../../runtime/runtime.c");

#[repr(u8)]
enum Tag {
    Tuple = 0,
    Closure = 247,
    Env = 248,
    String = 252,
}

pub struct Compiler<'ctx> {
    trees: HashMap<ast::Span, patternmatch::Tree>,
    types: Vec<ast::TypeDecl>,
    context: &'ctx Context,
    module: Module<'ctx>,
    builder: Builder<'ctx>,
    miniml_alloc: FunctionValue<'ctx>,
    miniml_equal: FunctionValue<'ctx>,
    miniml_string_concat: FunctionValue<'ctx>,
    miniml_print_char: FunctionValue<'ctx>,
    miniml_print_string: FunctionValue<'ctx>,
    miniml_print_string_escaped: FunctionValue<'ctx>,
    miniml_len: FunctionValue<'ctx>,
    miniml_unit_to_str: FunctionValue<'ctx>,
    miniml_bool_to_str: FunctionValue<'ctx>,
    miniml_char_to_str: FunctionValue<'ctx>,
    miniml_int_to_str: FunctionValue<'ctx>,
    cons_fn: FunctionValue<'ctx>,
    closure_fn: FunctionValue<'ctx>,
    printf: FunctionValue<'ctx>,
    builtin_print: FunctionValue<'ctx>,
    builtin_len: FunctionValue<'ctx>,
    builtin_unit_to_str: FunctionValue<'ctx>,
    builtin_bool_to_str: FunctionValue<'ctx>,
    builtin_char_to_str: FunctionValue<'ctx>,
    builtin_int_to_str: FunctionValue<'ctx>,
    word: IntType<'ctx>,
    ptr_type: PointerType<'ctx>,
    list_type: StructType<'ctx>,
    closure_type: StructType<'ctx>,
    lambda_type: FunctionType<'ctx>,
    arg_stack: GlobalValue<'ctx>,
    env_slot: Option<PointerValue<'ctx>>,
    /// `None` means the host
    target_triple: Option<String>,
    opt_level: OptimizationLevel,
}

impl<'ctx> Compiler<'ctx> {
    pub fn new(
        context: &'ctx Context,
        name: &str,
        trees: HashMap<ast::Span, patternmatch::Tree>,
        types: Vec<ast::TypeDecl>,
    ) -> Result<Self> {
        let module = context.create_module(name);

        let word = context.i64_type();
        let ptr_type = context.ptr_type(AddressSpace::default());

        // second param should be i8, but ABI could potentially fill with extra data
        // so passing full word is slightly easier, but also slightly misleading
        // declare ptr @miniml_alloc(i64, i64)
        let miniml_alloc = module.add_function(
            "miniml_alloc",
            ptr_type.fn_type(&[word.into(), word.into()], false),
            None,
        );
        // declare i64 @miniml_equal(i64, i64)
        let miniml_equal = module.add_function(
            "miniml_equal",
            word.fn_type(&[word.into(), word.into()], false),
            None,
        );
        // declare i64 @miniml_string_concat(i64, i64)
        let miniml_string_concat = module.add_function(
            "miniml_string_concat",
            word.fn_type(&[word.into(), word.into()], false),
            None,
        );
        // declare void @miniml_print_char(i64)
        let miniml_print_char = module.add_function(
            "miniml_print_char",
            context.void_type().fn_type(&[word.into()], false),
            None,
        );
        // declare void @miniml_print_string(i64)
        let miniml_print_string = module.add_function(
            "miniml_print_string",
            context.void_type().fn_type(&[word.into()], false),
            None,
        );
        // declare void @miniml_print_string_escaped(i64)
        let miniml_print_string_escaped = module.add_function(
            "miniml_print_string_escaped",
            context.void_type().fn_type(&[word.into()], false),
            None,
        );
        // declare i64 @miniml_len(i64)
        let miniml_len =
            module.add_function("miniml_len", word.fn_type(&[word.into()], false), None);
        // declare i64 @miniml_unit_to_str()
        let miniml_unit_to_str =
            module.add_function("miniml_unit_to_str", word.fn_type(&[], false), None);
        // declare i64 @miniml_bool_to_str(i64)
        let miniml_bool_to_str = module.add_function(
            "miniml_bool_to_str",
            word.fn_type(&[word.into()], false),
            None,
        );
        // declare i64 @miniml_char_to_str(i64)
        let miniml_char_to_str = module.add_function(
            "miniml_char_to_str",
            word.fn_type(&[word.into()], false),
            None,
        );
        // declare i64 @miniml_int_to_str(i64)
        let miniml_int_to_str = module.add_function(
            "miniml_int_to_str",
            word.fn_type(&[word.into()], false),
            None,
        );
        // declare i32 @printf(ptr, ...)
        let printf = module.add_function(
            "printf",
            context.i32_type().fn_type(&[ptr_type.into()], true),
            None,
        );

        let list_type = context.opaque_struct_type("list");
        list_type.set_body(&[word.into(), ptr_type.into()], false);

        let cons_fn = module.add_function(
            "cons",
            ptr_type.fn_type(&[word.into(), ptr_type.into()], false),
            Some(Linkage::Internal),
        );

        let closure_type = context.opaque_struct_type("closure");
        closure_type.set_body(&[ptr_type.into(), ptr_type.into()], false);

        let closure_fn = module.add_function(
            "closure",
            ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false),
            Some(Linkage::Internal),
        );

        let lambda_type = word.fn_type(&[ptr_type.into()], false);

        let builtin_print = module.add_function("print", lambda_type, Some(Linkage::Internal));
        builtin_print.set_call_conventions(TAIL_CC);

        let builtin_len = module.add_function("len", lambda_type, Some(Linkage::Internal));
        builtin_len.set_call_conventions(TAIL_CC);

        let builtin_unit_to_str =
            module.add_function("unit_to_str", lambda_type, Some(Linkage::Internal));
        builtin_unit_to_str.set_call_conventions(TAIL_CC);

        let builtin_bool_to_str =
            module.add_function("bool_to_str", lambda_type, Some(Linkage::Internal));
        builtin_bool_to_str.set_call_conventions(TAIL_CC);

        let builtin_char_to_str =
            module.add_function("char_to_str", lambda_type, Some(Linkage::Internal));
        builtin_char_to_str.set_call_conventions(TAIL_CC);

        let builtin_int_to_str =
            module.add_function("int_to_str", lambda_type, Some(Linkage::Internal));
        builtin_int_to_str.set_call_conventions(TAIL_CC);

        let arg_stack = module.add_global(ptr_type, None, "miniml_args");

        let builder = context.create_builder();

        let mut c = Self {
            trees,
            types,
            context,
            module,
            builder,
            miniml_alloc,
            miniml_equal,
            miniml_string_concat,
            miniml_print_char,
            miniml_print_string,
            miniml_print_string_escaped,
            miniml_len,
            miniml_unit_to_str,
            miniml_bool_to_str,
            miniml_char_to_str,
            miniml_int_to_str,
            cons_fn,
            closure_fn,
            printf,
            builtin_print,
            builtin_len,
            builtin_unit_to_str,
            builtin_bool_to_str,
            builtin_char_to_str,
            builtin_int_to_str,
            word,
            ptr_type,
            list_type,
            closure_type,
            lambda_type,
            arg_stack,
            env_slot: None,
            target_triple: None,
            opt_level: OptimizationLevel::None,
        };
        c.initialize()?;
        Ok(c)
    }

    fn initialize(&mut self) -> Result<()> {
        self.define_cons()?;
        self.define_closure()?;
        self.define_print()?;
        self.define_builtin(self.builtin_len, self.miniml_len)?;
        self.define_unit_to_str()?;
        self.define_builtin(self.builtin_bool_to_str, self.miniml_bool_to_str)?;
        self.define_builtin(self.builtin_char_to_str, self.miniml_char_to_str)?;
        self.define_builtin(self.builtin_int_to_str, self.miniml_int_to_str)?;
        Ok(())
    }

    pub fn compile(&mut self, program: &ast::Expr, kind: &ResultKind, label: &str) -> Result<()> {
        // define i64 @miniml_main() { entry: ... }
        let main = self
            .module
            .add_function("miniml_main", self.word.fn_type(&[], false), None);
        self.builder
            .position_at_end(self.context.append_basic_block(main, "entry"));
        self.env_slot = Some(self.new_env_slot(self.ptr_type.const_null())?);

        let result = self.compile_expression(program)?;
        match kind {
            #[cfg(test)]
            ResultKind::ExitCode => {
                let exit_code = self.unbox_int(result)?;
                self.builder.build_return(Some(&exit_code))?;
                return Ok(());
            }
            ResultKind::Never => {}
            kind => {
                self.call_printf(&label.replace('%', "%%"), &[])?;
                self.print_value(kind, result)?;
                self.call_printf("\n", &[])?;
            }
        }

        self.builder.build_return(Some(&self.word.const_zero()))?;
        Ok(())
    }

    fn print_value(&self, kind: &ResultKind, value: IntValue<'ctx>) -> Result<()> {
        match kind {
            ResultKind::Unit => self.call_printf("()", &[])?,
            ResultKind::Bool => {
                let n = self.unbox_int(value)?;
                let is_true = self.builder.build_int_compare(
                    IntPredicate::NE,
                    n,
                    self.word.const_zero(),
                    "is_true",
                )?;
                let t = self.builder.build_global_string_ptr("true", "true.str")?;
                let f = self.builder.build_global_string_ptr("false", "false.str")?;
                let s = self.builder.build_select(
                    is_true,
                    t.as_pointer_value(),
                    f.as_pointer_value(),
                    "bool.str",
                )?;
                self.call_printf("%s", &[s.into()])?;
            }
            ResultKind::Char => {
                let c = self.unbox_int(value)?;
                self.call_printf("'", &[])?;
                self.builder
                    .build_direct_call(self.miniml_print_char, &[c.into()], "")?;
                self.call_printf("'", &[])?;
            }
            ResultKind::Int => {
                let n = self.unbox_int(value)?;
                self.call_printf("%lld", &[n.into()])?;
            }
            ResultKind::String => {
                self.call_printf("\"", &[])?;
                self.builder.build_direct_call(
                    self.miniml_print_string_escaped,
                    &[value.into()],
                    "",
                )?;
                self.call_printf("\"", &[])?;
            }
            ResultKind::Tuple(kinds) => {
                let tuple = self
                    .builder
                    .build_int_to_ptr(value, self.ptr_type, "tuple")?;
                self.call_printf("(", &[])?;
                for (i, kind) in kinds.iter().enumerate() {
                    if i > 0 {
                        self.call_printf(", ", &[])?;
                    }
                    let field = unsafe {
                        self.builder.build_gep(
                            self.word,
                            tuple,
                            &[self.word.const_int(i as u64, false)],
                            "tuple.field",
                        )?
                    };
                    let field = self
                        .builder
                        .build_load(self.word, field, "field")?
                        .into_int_value();
                    self.print_value(kind, field)?;
                }
                self.call_printf(")", &[])?;
            }
            ResultKind::Named(id, kinds) => {
                let print_fn = self.print_named_fn(*id, kinds)?;
                self.builder
                    .build_direct_call(print_fn, &[value.into()], "")?;
            }
            ResultKind::Function => self.call_printf("<fun>", &[])?,
            ResultKind::Never => unreachable!("handled in compile"),
            #[cfg(test)]
            ResultKind::ExitCode => unreachable!("handled in compile"),
        }
        Ok(())
    }

    /// One print function per instantiated type
    fn print_named_fn(
        &self,
        id: typecheck::TypeId,
        kinds: &[ResultKind],
    ) -> Result<FunctionValue<'ctx>> {
        // e.g.
        // type List a               -> TypeId = 0
        //   = Nil                   -> index  = 0
        //   | Cons a (List a)       -> index  = 1
        // List::Nil                 -> stored as immediate (index << 1) | 1
        // List::Cons <value> <list> -> store as object with header tag = index
        let name = format!("print.T_{}.{kinds:?}", id.0);
        // If we've already defined e.g. @"print.T_0.[Int]", skip
        if let Some(f) = self.module.get_function(&name) {
            return Ok(f);
        }
        let decl = self
            .types
            .get(id.0)
            .ok_or(CodegenError::UnknownType { id: id.0 })?;
        let f = self.module.add_function(
            &name,
            self.context.void_type().fn_type(&[self.word.into()], false),
            Some(Linkage::Internal),
        );
        // save block position for caller
        let previous = self.builder.get_insert_block();

        let entry = self.context.append_basic_block(f, "entry");
        let immediate = self.context.append_basic_block(f, "immediate");
        let pointer = self.context.append_basic_block(f, "pointer");
        let dispatch = self.context.append_basic_block(f, "dispatch");
        let invalid = self.context.append_basic_block(f, "invalid");

        // test for nullary constructors/immediates
        self.builder.position_at_end(entry);
        let value = f.get_first_param().unwrap().into_int_value();
        let one = self.word.const_int(1, false);
        let low_bit = self.builder.build_and(value, one, "low_bit")?;
        let is_immediate = self.builder.build_int_compare(
            IntPredicate::NE,
            low_bit,
            self.word.const_zero(),
            "is_imm",
        )?;
        self.builder
            .build_conditional_branch(is_immediate, immediate, pointer)?;

        self.builder.position_at_end(immediate);
        let imm_index = self
            .builder
            .build_right_shift(value, one, false, "imm.index")?;
        self.builder.build_unconditional_branch(dispatch)?;

        self.builder.position_at_end(pointer);
        let tag = self.load_tag(value)?;
        self.builder.build_unconditional_branch(dispatch)?;

        // index = phi <immediate> <object>
        self.builder.position_at_end(dispatch);
        let index = self.builder.build_phi(self.word, "index")?;
        index.add_incoming(&[(&imm_index, immediate), (&tag, pointer)]);

        // match on index
        let mut cases = Vec::new();
        for (i, _) in decl.constructors.iter().enumerate() {
            let case = self.context.append_basic_block(f, &format!("ctor.{i}"));
            cases.push((self.word.const_int(i as u64, false), case));
        }
        self.builder
            .build_switch(index.as_basic_value().into_int_value(), invalid, &cases)?;

        for ((i, (_, fields)), (_, case)) in decl.constructors.iter().enumerate().zip(&cases) {
            self.builder.position_at_end(*case);
            let field_kinds: Vec<ResultKind> =
                fields.iter().map(|tp| result_kind(tp, kinds)).collect();
            // this can happen legitimately
            // e.g. Option::None : ∀α.Option α which is treated as None -> <immediate>, and Some α -> Never
            if field_kinds.contains(&ResultKind::Never) {
                self.builder.build_unreachable()?;
                continue;
            }
            self.call_printf(&format!("T_{}::[{i}]", id.0), &[])?;
            if !field_kinds.is_empty() {
                let ptr = self
                    .builder
                    .build_int_to_ptr(value, self.ptr_type, "fields")?;
                self.call_printf("(", &[])?;
                for (j, kind) in field_kinds.iter().enumerate() {
                    if j > 0 {
                        self.call_printf(", ", &[])?;
                    }
                    let field = unsafe {
                        self.builder.build_gep(
                            self.word,
                            ptr,
                            &[self.word.const_int(j as u64, false)],
                            "field.ptr",
                        )?
                    };
                    let field = self
                        .builder
                        .build_load(self.word, field, "field")?
                        .into_int_value();
                    self.print_value(kind, field)?;
                }
                self.call_printf(")", &[])?;
            }
            self.builder.build_return(None)?;
        }

        self.builder.position_at_end(invalid);
        self.builder.build_unreachable()?;

        if let Some(previous) = previous {
            self.builder.position_at_end(previous);
        }
        Ok(f)
    }

    fn call_printf(&self, format: &str, args: &[BasicMetadataValueEnum<'ctx>]) -> Result<()> {
        let format = self.builder.build_global_string_ptr(format, "format")?;
        let mut all_args = vec![format.as_pointer_value().into()];
        all_args.extend_from_slice(args);
        self.builder.build_direct_call(self.printf, &all_args, "")?;
        Ok(())
    }
}
