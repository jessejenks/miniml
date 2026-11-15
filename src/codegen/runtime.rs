use inkwell::values::{BasicMetadataValueEnum, FunctionValue, IntValue};

use super::{CodegenError, Compiler, Result};
use crate::ast;

impl<'ctx> Compiler<'ctx> {
    pub(super) fn define_cons(&mut self) -> Result<()> {
        let cons = self.cons_fn;
        self.builder
            .position_at_end(self.context.append_basic_block(cons, "entry"));

        let head = cons.get_nth_param(0).unwrap().into_int_value();
        let tail = cons.get_nth_param(1).unwrap().into_pointer_value();
        head.set_name("head");
        tail.set_name("tail");

        let cell = self
            .builder
            .build_direct_call(
                self.miniml_alloc,
                &[
                    self.word.const_int(2, false).into(),
                    self.word.const_int(248, false).into(),
                ],
                "cell",
            )? // 0 = env tag
            .try_as_basic_value()
            .expect_basic("malloc returns a value")
            .into_pointer_value();

        // cell.head = head
        let head_ptr = self
            .builder
            .build_struct_gep(self.list_type, cell, 0, "cell.head")?;
        self.builder.build_store(head_ptr, head)?;

        // cell.tail = tail
        let tail_ptr = self
            .builder
            .build_struct_gep(self.list_type, cell, 1, "cell.tail")?;
        self.builder.build_store(tail_ptr, tail)?;

        self.builder.build_return(Some(&cell))?;
        Ok(())
    }

    pub(super) fn define_closure(&mut self) -> Result<()> {
        let f = self.closure_fn;
        self.builder
            .position_at_end(self.context.append_basic_block(f, "entry"));

        let code = f.get_nth_param(0).unwrap().into_pointer_value();
        let env = f.get_nth_param(1).unwrap().into_pointer_value();
        code.set_name("code");
        env.set_name("env");

        let closure = self
            .builder
            .build_direct_call(
                self.miniml_alloc,
                &[
                    self.word.const_int(2, false).into(),
                    self.word.const_int(247, false).into(),
                ],
                "closure",
            )? // closure tag
            .try_as_basic_value()
            .expect_basic("malloc returns a value")
            .into_pointer_value();

        let code_ptr =
            self.builder
                .build_struct_gep(self.closure_type, closure, 0, "closure.code")?;
        self.builder.build_store(code_ptr, code)?;
        let env_ptr =
            self.builder
                .build_struct_gep(self.closure_type, closure, 1, "closure.env")?;
        self.builder.build_store(env_ptr, env)?;

        self.builder.build_return(Some(&closure))?;
        Ok(())
    }

    /// define internal tailcc i64 @print(ptr %env)
    /// Effectively `fun s => { printf("%s\n", s); () }`
    pub(super) fn define_print(&mut self) -> Result<()> {
        self.begin_builtin(self.builtin_print)?;

        let s = self.access(0)?;
        self.builder
            .build_direct_call(self.miniml_print_string, &[s.into()], "")?;
        self.call_printf("\n", &[])?;
        let unit = self.box_const(0)?;
        self.ret(unit)?;

        self.end_builtin();
        Ok(())
    }

    pub(super) fn define_builtin(
        &mut self,
        builtin_func: FunctionValue<'ctx>,
        runtime_func: FunctionValue<'ctx>,
    ) -> Result<()> {
        self.begin_builtin(builtin_func)?;

        let x = self.access(0)?;
        let r = self.call_runtime_func(runtime_func, &[x.into()])?;
        self.ret(r)?;

        self.end_builtin();
        Ok(())
    }

    pub(super) fn define_unit_to_str(&mut self) -> Result<()> {
        self.begin_builtin(self.builtin_unit_to_str)?;

        // runtime function doesn't need an argument
        let r = self.call_runtime_func(self.miniml_unit_to_str, &[])?;
        self.ret(r)?;

        self.end_builtin();
        Ok(())
    }

    fn begin_builtin(&mut self, f: FunctionValue<'ctx>) -> Result<()> {
        self.builder
            .position_at_end(self.context.append_basic_block(f, "entry"));
        let env = f.get_first_param().unwrap().into_pointer_value();
        env.set_name("env");
        self.env_slot = Some(self.new_env_slot(env)?);
        Ok(())
    }

    fn end_builtin(&mut self) {
        self.env_slot = None;
    }

    fn call_runtime_func(
        &self,
        runtime_func: FunctionValue<'ctx>,
        args: &[BasicMetadataValueEnum<'ctx>],
    ) -> Result<IntValue<'ctx>> {
        Ok(self
            .builder
            .build_direct_call(runtime_func, args, "r")?
            .try_as_basic_value()
            .expect_basic("runtime function returns a value")
            .into_int_value())
    }

    pub(super) fn free_variable(&self, name: &str, span: ast::Span) -> Result<IntValue<'ctx>> {
        match name {
            "print" => self.cur(self.builtin_print),
            "len" => self.cur(self.builtin_len),
            "unitToStr" => self.cur(self.builtin_unit_to_str),
            "boolToStr" => self.cur(self.builtin_bool_to_str),
            "charToStr" => self.cur(self.builtin_char_to_str),
            "intToStr" => self.cur(self.builtin_int_to_str),
            _ => Err(CodegenError::UnknownVariable {
                name: name.to_string(),
                span: span.into(),
            }),
        }
    }
}
