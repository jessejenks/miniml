use inkwell::{
    IntPredicate,
    basic_block::BasicBlock,
    module::Linkage,
    values::{CallSiteValue, FunctionValue, IntValue, LLVMTailCallKind, PointerValue},
};

use super::{Compiler, Result, TAIL_CC};

impl<'ctx> Compiler<'ctx> {
    /// `%cell = call ptr @cons(i64 head, ptr tail)`
    fn cons(&self, head: IntValue<'ctx>, tail: PointerValue<'ctx>) -> Result<PointerValue<'ctx>> {
        Ok(self
            .builder
            .build_direct_call(self.cons_fn, &[head.into(), tail.into()], "cell")?
            .try_as_basic_value()
            .expect_basic("cons returns a value")
            .into_pointer_value())
    }

    /// `ARGS.push(value)`
    pub(super) fn push(&self, value: IntValue<'ctx>) -> Result<()> {
        let args = self.arg_stack.as_pointer_value();
        let stack_pointer = self
            .builder
            .build_load(self.ptr_type, args, "args.sp")?
            .into_pointer_value();
        self.builder.build_store(stack_pointer, value)?;
        let next = unsafe {
            self.builder.build_gep(
                self.word,
                stack_pointer,
                &[self.word.const_int(1, false)],
                "args.next",
            )
        }?;
        self.builder.build_store(args, next)?;
        Ok(())
    }

    /// `ARGS.push(mark)`
    pub(super) fn pushmark(&self) -> Result<()> {
        self.push(self.word.const_zero())
    }

    /// address of value at top of argument stack
    fn args_top(&self) -> Result<PointerValue<'ctx>> {
        let args = self.arg_stack.as_pointer_value();
        let stack_pointer = self
            .builder
            .build_load(self.ptr_type, args, "args.sp")?
            .into_pointer_value();
        Ok(unsafe {
            self.builder.build_gep(
                self.word,
                stack_pointer,
                &[self.word.const_all_ones()], // 1...1b = -1
                "args.top",
            )
        }?)
    }

    /// `ARGS.peek()`
    fn peek_args(&self) -> Result<IntValue<'ctx>> {
        let top = self.args_top()?;
        Ok(self
            .builder
            .build_load(self.word, top, "arg")?
            .into_int_value())
    }

    /// `ARGS.pop()`
    fn pop_args(&self) -> Result<()> {
        let args = self.arg_stack.as_pointer_value();
        let top = self.args_top()?;
        self.builder.build_store(args, top)?;
        Ok(())
    }

    fn head(&self, list: PointerValue<'ctx>) -> Result<IntValue<'ctx>> {
        let p = self
            .builder
            .build_struct_gep(self.list_type, list, 0, "head.ptr")?;
        Ok(self
            .builder
            .build_load(self.word, p, "head")?
            .into_int_value())
    }

    fn tail(&self, list: PointerValue<'ctx>) -> Result<PointerValue<'ctx>> {
        let p = self
            .builder
            .build_struct_gep(self.list_type, list, 1, "tail.ptr")?;
        Ok(self
            .builder
            .build_load(self.ptr_type, p, "tail")?
            .into_pointer_value())
    }

    fn env_slot(&self) -> PointerValue<'ctx> {
        self.env_slot.expect("compiling inside a function")
    }

    fn load_env(&self) -> Result<PointerValue<'ctx>> {
        Ok(self
            .builder
            .build_load(self.ptr_type, self.env_slot(), "env")?
            .into_pointer_value())
    }

    fn store_env(&self, env: PointerValue<'ctx>) -> Result<()> {
        self.builder.build_store(self.env_slot(), env)?;
        Ok(())
    }

    /// %env.slot = alloca ptr; store initial
    pub(super) fn new_env_slot(&self, initial: PointerValue<'ctx>) -> Result<PointerValue<'ctx>> {
        let slot = self.builder.build_alloca(self.ptr_type, "env.slot")?;
        self.builder.build_store(slot, initial)?;
        Ok(slot)
    }

    /// Creates `define internal tailcc i64 @lambda(ptr %env)`
    pub(super) fn begin_function(
        &mut self,
    ) -> Result<(
        FunctionValue<'ctx>,
        BasicBlock<'ctx>,
        Option<PointerValue<'ctx>>,
    )> {
        let outer_block = self
            .builder
            .get_insert_block()
            .expect("builder is positioned");
        let outer_env = self.env_slot;

        let function =
            self.module
                .add_function("lambda", self.lambda_type, Some(Linkage::Internal));
        function.set_call_conventions(TAIL_CC);
        self.builder
            .position_at_end(self.context.append_basic_block(function, "entry"));

        let env = function.get_first_param().unwrap().into_pointer_value();
        env.set_name("env");
        self.env_slot = Some(self.new_env_slot(env)?);

        Ok((function, outer_block, outer_env))
    }

    pub(super) fn end_function(
        &mut self,
        block: BasicBlock<'ctx>,
        env: Option<PointerValue<'ctx>>,
    ) {
        self.builder.position_at_end(block);
        self.env_slot = env;
    }

    pub(super) fn access(&self, n: i64) -> Result<IntValue<'ctx>> {
        let mut list = self.load_env()?;
        for _ in 0..n {
            list = self.tail(list)?;
        }
        self.head(list)
    }

    /// env = cons(accu, env)
    pub(super) fn let_in(&self, value: IntValue<'ctx>) -> Result<()> {
        let env = self.load_env()?;
        let cell = self.cons(value, env)?;
        self.store_env(cell)
    }

    /// env = cons(dummy, env)
    pub(super) fn dummy(&self) -> Result<()> {
        self.let_in(self.word.const_zero())
    }

    /// env.head = accu
    pub(super) fn update(&self, value: IntValue<'ctx>) -> Result<()> {
        let env = self.load_env()?;
        let head_ptr = self
            .builder
            .build_struct_gep(self.list_type, env, 0, "env.head")?;
        self.builder.build_store(head_ptr, value)?;
        Ok(())
    }

    /// env = env.tail
    pub(super) fn end_let(&self) -> Result<()> {
        let env = self.load_env()?;
        let tail = self.tail(env)?;
        self.store_env(tail)
    }

    pub(super) fn cur(&self, function: FunctionValue<'ctx>) -> Result<IntValue<'ctx>> {
        let code = function.as_global_value().as_pointer_value();
        let env = self.load_env()?;
        let closure = self
            .builder
            .build_direct_call(self.closure_fn, &[code.into(), env.into()], "closure")?
            .try_as_basic_value()
            .expect_basic("closure returns a value")
            .into_pointer_value();
        Ok(self
            .builder
            .build_ptr_to_int(closure, self.word, "closure.value")?)
    }

    pub(super) fn apply(&self, closure: IntValue<'ctx>) -> Result<IntValue<'ctx>> {
        let call = self.call_closure(closure)?;
        Ok(call
            .try_as_basic_value()
            .expect_basic("lambdas return a value")
            .into_int_value())
    }

    /// Ends the current block.
    pub(super) fn appterm(&self, closure: IntValue<'ctx>) -> Result<()> {
        let call = self.call_closure(closure)?;
        self.return_tail_call(call)
    }

    fn call_closure(&self, closure: IntValue<'ctx>) -> Result<CallSiteValue<'ctx>> {
        let closure = self
            .builder
            .build_int_to_ptr(closure, self.ptr_type, "closure")?;

        let arg = self.peek_args()?;
        let env_ptr =
            self.builder
                .build_struct_gep(self.closure_type, closure, 1, "closure.env.ptr")?;
        let closure_env = self
            .builder
            .build_load(self.ptr_type, env_ptr, "closure.env")?
            .into_pointer_value();
        let new_env = self.cons(arg, closure_env)?;

        self.pop_args()?;

        let code_ptr =
            self.builder
                .build_struct_gep(self.closure_type, closure, 0, "closure.code.ptr")?;
        let code = self
            .builder
            .build_load(self.ptr_type, code_ptr, "closure.code")?
            .into_pointer_value();

        let call = self.builder.build_indirect_call(
            self.lambda_type,
            code,
            &[new_env.into()],
            "result",
        )?;
        call.set_call_convention(TAIL_CC);
        Ok(call)
    }

    fn return_tail_call(&self, call: CallSiteValue<'ctx>) -> Result<()> {
        call.set_tail_call_kind(LLVMTailCallKind::LLVMTailCallKindMustTail);
        let result = call
            .try_as_basic_value()
            .expect_basic("lambdas return a value")
            .into_int_value();
        self.builder.build_return(Some(&result))?;
        Ok(())
    }

    fn branch_on_mark(&self, name: &str) -> Result<(BasicBlock<'ctx>, BasicBlock<'ctx>)> {
        let head = self.peek_args()?;
        let is_mark = self.builder.build_int_compare(
            IntPredicate::EQ,
            head,
            self.word.const_zero(),
            "is_mark",
        )?;
        let function = self
            .builder
            .get_insert_block()
            .and_then(BasicBlock::get_parent)
            .expect("builder is inside a function");
        let mark = self
            .context
            .append_basic_block(function, &format!("{name}.mark"));
        let arg = self
            .context
            .append_basic_block(function, &format!("{name}.arg"));
        self.builder.build_conditional_branch(is_mark, mark, arg)?;
        Ok((mark, arg))
    }

    pub(super) fn grab(&self, function: FunctionValue<'ctx>) -> Result<()> {
        let (mark, arg) = self.branch_on_mark("grab")?;

        self.builder.position_at_end(mark);
        self.pop_args()?;
        let closure = self.cur(function)?;
        self.builder.build_return(Some(&closure))?;

        self.builder.position_at_end(arg);
        let value = self.peek_args()?;
        let new_env = self.cons(value, self.load_env()?)?;
        self.pop_args()?;
        let call = self
            .builder
            .build_direct_call(function, &[new_env.into()], "result")?;
        call.set_call_convention(TAIL_CC);
        self.return_tail_call(call)
    }

    pub(super) fn ret(&self, value: IntValue<'ctx>) -> Result<()> {
        let (mark, arg) = self.branch_on_mark("ret")?;

        self.builder.position_at_end(mark);
        self.pop_args()?;
        self.builder.build_return(Some(&value))?;

        self.builder.position_at_end(arg);
        self.appterm(value)
    }

    pub(super) fn insert_block(&self) -> BasicBlock<'ctx> {
        self.builder
            .get_insert_block()
            .expect("builder is positioned")
    }

    pub(super) fn branch_on_bool(
        &self,
        condition: IntValue<'ctx>,
        name: &str,
    ) -> Result<(BasicBlock<'ctx>, BasicBlock<'ctx>)> {
        let unboxed = self.unbox_int(condition)?;
        let is_true = self.builder.build_int_compare(
            IntPredicate::NE,
            unboxed,
            self.word.const_zero(),
            "is_true",
        )?;
        let function = self
            .insert_block()
            .get_parent()
            .expect("builder is inside a function");
        let then_block = self
            .context
            .append_basic_block(function, &format!("{name}.then"));
        let else_block = self
            .context
            .append_basic_block(function, &format!("{name}.else"));
        self.builder
            .build_conditional_branch(is_true, then_block, else_block)?;
        Ok((then_block, else_block))
    }

    pub(super) fn merge(
        &self,
        name: &str,
        branches: [(IntValue<'ctx>, BasicBlock<'ctx>); 2],
    ) -> Result<IntValue<'ctx>> {
        let function = self
            .insert_block()
            .get_parent()
            .expect("builder is inside a function");
        let merge = self
            .context
            .append_basic_block(function, &format!("{name}.end"));
        for (_, block) in branches {
            self.builder.position_at_end(block);
            self.builder.build_unconditional_branch(merge)?;
        }
        self.builder.position_at_end(merge);
        let phi = self.builder.build_phi(self.word, name)?;
        for (value, block) in branches {
            phi.add_incoming(&[(&value, block)]);
        }
        Ok(phi.as_basic_value().into_int_value())
    }
}
