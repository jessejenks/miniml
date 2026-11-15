use inkwell::{
    AddressSpace, IntPredicate, OptimizationLevel,
    builder::{Builder, BuilderError},
    context::Context,
    module::{Linkage, Module},
    support::LLVMString,
    types::{FunctionType, IntType, PointerType, StructType},
    values::{BasicMetadataValueEnum, FunctionValue, GlobalValue, PointerValue},
};
use miette::{Diagnostic, SourceSpan};
use thiserror::Error;

use crate::ast;

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
}

impl From<LLVMString> for CodegenError {
    fn from(e: LLVMString) -> Self {
        CodegenError::Llvm(e.to_string())
    }
}

type Result<T> = std::result::Result<T, CodegenError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultKind {
    Unit,
    Bool,
    Char,
    Int,
    String,
    Function,
    /// no return value expected
    Never,
    /// always return the unboxed result as the exit code
    ExitCode,
}

const TAIL_CC: u32 = 18;

const RUNTIME: &str = include_str!("../../runtime/runtime.c");

pub struct Compiler<'ctx> {
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
    pub fn new(context: &'ctx Context, name: &str) -> Result<Self> {
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

    pub fn compile(&mut self, program: &ast::Expr, kind: ResultKind, label: &str) -> Result<()> {
        // define i64 @miniml_main() { entry: ... }
        let main = self
            .module
            .add_function("miniml_main", self.word.fn_type(&[], false), None);
        self.builder
            .position_at_end(self.context.append_basic_block(main, "entry"));
        self.env_slot = Some(self.new_env_slot(self.ptr_type.const_null())?);

        let result = self.compile_expression(program)?;
        let label = label.replace('%', "%%");
        match kind {
            ResultKind::ExitCode => {
                let exit_code = self.unbox_int(result)?;
                self.builder.build_return(Some(&exit_code))?;
                return Ok(());
            }
            ResultKind::Unit => self.call_printf(&format!("{label}()\n"), &[])?,
            ResultKind::Bool => {
                let n = self.unbox_int(result)?;
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
                self.call_printf(&format!("{label}%s\n"), &[s.into()])?;
            }
            ResultKind::Char => {
                let c = self.unbox_int(result)?;
                self.call_printf(&format!("{label}'"), &[])?;
                self.builder
                    .build_direct_call(self.miniml_print_char, &[c.into()], "")?;
                self.call_printf("'\n", &[])?;
            }
            ResultKind::Int => {
                let n = self.unbox_int(result)?;
                self.call_printf(&format!("{label}%lld\n"), &[n.into()])?;
            }
            ResultKind::String => {
                self.call_printf(&format!("{label}\""), &[])?;
                self.builder.build_direct_call(
                    self.miniml_print_string_escaped,
                    &[result.into()],
                    "",
                )?;
                self.call_printf("\"\n", &[])?;
            }
            ResultKind::Function => self.call_printf(&format!("{label}<fun>\n"), &[])?,
            ResultKind::Never => {}
        }
        self.builder.build_return(Some(&self.word.const_zero()))?;
        Ok(())
    }

    fn call_printf(&self, format: &str, args: &[BasicMetadataValueEnum<'ctx>]) -> Result<()> {
        let format = self.builder.build_global_string_ptr(format, "format")?;
        let mut all_args = vec![format.as_pointer_value().into()];
        all_args.extend_from_slice(args);
        self.builder.build_direct_call(self.printf, &all_args, "")?;
        Ok(())
    }
}
