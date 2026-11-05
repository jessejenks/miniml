use inkwell::{
    AddressSpace, IntPredicate,
    basic_block::BasicBlock,
    builder::{Builder, BuilderError},
    context::Context,
    module::{Linkage, Module},
    types::{FunctionType, IntType, PointerType, StructType},
    values::{CallSiteValue, FunctionValue, GlobalValue, IntValue, LLVMTailCallKind, PointerValue},
};

use crate::ast;

type Result<T> = std::result::Result<T, BuilderError>;

const TAIL_CC: u32 = 18;

pub struct Compiler<'ctx> {
    context: &'ctx Context,
    module: Module<'ctx>,
    builder: Builder<'ctx>,
    malloc: FunctionValue<'ctx>,
    cons_fn: FunctionValue<'ctx>,
    closure_fn: FunctionValue<'ctx>,
    word: IntType<'ctx>,
    ptr_type: PointerType<'ctx>,
    list_type: StructType<'ctx>,
    closure_type: StructType<'ctx>,
    lambda_type: FunctionType<'ctx>,
    arg_stack: GlobalValue<'ctx>,
    env_slot: Option<PointerValue<'ctx>>,
}

impl<'ctx> Compiler<'ctx> {
    pub fn new(context: &'ctx Context, name: &str) -> Result<Self> {
        let module = context.create_module(name);

        let word = context.i64_type();
        let ptr_type = context.ptr_type(AddressSpace::default());

        let malloc = module.add_function("malloc", ptr_type.fn_type(&[word.into()], false), None);

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

        let arg_stack = module.add_global(ptr_type, None, "ARGS");
        arg_stack.set_initializer(&ptr_type.const_null());
        arg_stack.set_linkage(Linkage::Internal);

        let builder = context.create_builder();

        let mut c = Self {
            context,
            module,
            builder,
            malloc,
            cons_fn,
            closure_fn,
            word,
            ptr_type,
            list_type,
            closure_type,
            lambda_type,
            arg_stack,
            env_slot: None,
        };
        c.initialize()?;
        Ok(c)
    }

    fn initialize(&mut self) -> Result<()> {
        self.define_cons()?;
        self.define_closure()?;
        Ok(())
    }

    fn define_cons(&mut self) -> Result<()> {
        let cons = self.cons_fn;
        self.builder
            .position_at_end(self.context.append_basic_block(cons, "entry"));

        let head = cons.get_nth_param(0).unwrap().into_int_value();
        let tail = cons.get_nth_param(1).unwrap().into_pointer_value();
        head.set_name("head");
        tail.set_name("tail");

        let list_size = self.list_type.size_of().expect("list type has body");
        let cell = self
            .builder
            .build_direct_call(self.malloc, &[list_size.into()], "cell")?
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

    fn define_closure(&mut self) -> Result<()> {
        let f = self.closure_fn;
        self.builder
            .position_at_end(self.context.append_basic_block(f, "entry"));

        let code = f.get_nth_param(0).unwrap().into_pointer_value();
        let env = f.get_nth_param(1).unwrap().into_pointer_value();
        code.set_name("code");
        env.set_name("env");

        let size = self.closure_type.size_of().expect("closure type has body");
        let closure = self
            .builder
            .build_direct_call(self.malloc, &[size.into()], "closure")?
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

    /// %cell = call ptr @cons(i64 head, ptr tail)
    fn cons(&self, head: IntValue<'ctx>, tail: PointerValue<'ctx>) -> Result<PointerValue<'ctx>> {
        Ok(self
            .builder
            .build_direct_call(self.cons_fn, &[head.into(), tail.into()], "cell")?
            .try_as_basic_value()
            .expect_basic("cons returns a value")
            .into_pointer_value())
    }

    /// ARGS = cons(value, ARGS)
    fn push(&self, value: IntValue<'ctx>) -> Result<()> {
        let args = self.arg_stack.as_pointer_value();
        let old = self
            .builder
            .build_load(self.ptr_type, args, "args")?
            .into_pointer_value();
        let cell = self.cons(value, old)?;
        self.builder.build_store(args, cell)?;
        Ok(())
    }

    /// ARGS = cons(mark, ARGS)
    fn pushmark(&self) -> Result<()> {
        self.push(self.word.const_zero())
    }

    /// ARGS = ARGS.tail
    fn pop_args(&self) -> Result<()> {
        let args = self.arg_stack.as_pointer_value();
        let cell = self
            .builder
            .build_load(self.ptr_type, args, "args")?
            .into_pointer_value();
        let tail_ptr = self
            .builder
            .build_struct_gep(self.list_type, cell, 1, "args.tail")?;
        let tail = self.builder.build_load(self.ptr_type, tail_ptr, "tail")?;
        self.builder.build_store(args, tail)?;
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

    fn load_args(&self) -> Result<PointerValue<'ctx>> {
        Ok(self
            .builder
            .build_load(self.ptr_type, self.arg_stack.as_pointer_value(), "args")?
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

    /// %env.slot = alloca ptr; store initial
    fn new_env_slot(&self, initial: PointerValue<'ctx>) -> Result<PointerValue<'ctx>> {
        let slot = self.builder.build_alloca(self.ptr_type, "env.slot")?;
        self.builder.build_store(slot, initial)?;
        Ok(slot)
    }

    /// Creates `define internal tailcc i64 @lambda(ptr %env)`
    fn begin_function(
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

    fn end_function(&mut self, block: BasicBlock<'ctx>, env: Option<PointerValue<'ctx>>) {
        self.builder.position_at_end(block);
        self.env_slot = env;
    }

    fn access(&self, n: i64) -> Result<IntValue<'ctx>> {
        let mut list = self.load_env()?;
        for _ in 0..n {
            list = self.tail(list)?;
        }
        self.head(list)
    }

    fn cur(&self, function: FunctionValue<'ctx>) -> Result<IntValue<'ctx>> {
        let code = function.as_global_value().as_pointer_value();
        let env = self.load_env()?;
        let closure = self
            .builder
            .build_direct_call(self.closure_fn, &[code.into(), env.into()], "closure")?
            .try_as_basic_value()
            .expect_basic("closure returns a value")
            .into_pointer_value();
        self.builder
            .build_ptr_to_int(closure, self.word, "closure.value")
    }

    fn apply(&self, closure: IntValue<'ctx>) -> Result<IntValue<'ctx>> {
        let call = self.call_closure(closure)?;
        Ok(call
            .try_as_basic_value()
            .expect_basic("lambdas return a value")
            .into_int_value())
    }

    /// Ends the current block.
    fn appterm(&self, closure: IntValue<'ctx>) -> Result<()> {
        let call = self.call_closure(closure)?;
        self.return_tail_call(call)
    }

    fn call_closure(&self, closure: IntValue<'ctx>) -> Result<CallSiteValue<'ctx>> {
        let closure = self
            .builder
            .build_int_to_ptr(closure, self.ptr_type, "closure")?;

        let arg = self.head(self.load_args()?)?;
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
        let head = self.head(self.load_args()?)?;
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

    fn grab(&self, function: FunctionValue<'ctx>) -> Result<()> {
        let (mark, arg) = self.branch_on_mark("grab")?;

        self.builder.position_at_end(mark);
        self.pop_args()?;
        let closure = self.cur(function)?;
        self.builder.build_return(Some(&closure))?;

        self.builder.position_at_end(arg);
        let value = self.head(self.load_args()?)?;
        let new_env = self.cons(value, self.load_env()?)?;
        self.pop_args()?;
        let call = self
            .builder
            .build_direct_call(function, &[new_env.into()], "result")?;
        call.set_call_convention(TAIL_CC);
        self.return_tail_call(call)
    }

    fn ret(&self, value: IntValue<'ctx>) -> Result<()> {
        let (mark, arg) = self.branch_on_mark("ret")?;

        self.builder.position_at_end(mark);
        self.pop_args()?;
        self.builder.build_return(Some(&value))?;

        self.builder.position_at_end(arg);
        self.appterm(value)
    }

    pub fn ir(&self) -> String {
        self.module.print_to_string().to_string()
    }

    pub fn compile(&mut self, program: &ast::Expr) -> Result<()> {
        // define i64 @main() { entry: ... }
        let main = self
            .module
            .add_function("main", self.word.fn_type(&[], false), None);
        self.builder
            .position_at_end(self.context.append_basic_block(main, "entry"));
        self.env_slot = Some(self.new_env_slot(self.ptr_type.const_null())?);

        // For now, return value is always assumed to be an int
        let result = self.compile_expression(program)?;
        let exit_code = self.unbox_int(result)?;
        self.builder.build_return(Some(&exit_code))?;
        Ok(())
    }

    fn compile_expression(&mut self, expr: &ast::Expr) -> Result<IntValue<'ctx>> {
        match &expr.kind {
            ast::ExprKind::Unit
            | ast::ExprKind::Bool(_)
            | ast::ExprKind::Int(_)
            | ast::ExprKind::Unary(..)
            | ast::ExprKind::Binary(..) => self.compile_primitive(expr),
            ast::ExprKind::BoundVariable(n) => self.access(*n),
            ast::ExprKind::FreeVariable(_) => todo!(),
            ast::ExprKind::Application(left, right) => {
                self.pushmark()?;
                let arg = self.compile_expression(right)?;
                self.push(arg)?;
                let closure = self.compile_expression(left)?;
                self.apply(closure)
            }
            ast::ExprKind::Abstraction(_, body) => {
                let (func, block, env) = self.begin_function()?;
                self.compile_expression_tail(body)?;
                self.end_function(block, env);
                self.cur(func)
            }
            ast::ExprKind::LetIn(is_rec, _, expr, body) => todo!(),
            ast::ExprKind::IfElse(condition, consequence, alternative) => todo!(),
        }
    }

    fn compile_expression_tail(&mut self, expr: &ast::Expr) -> Result<()> {
        match &expr.kind {
            ast::ExprKind::Unit
            | ast::ExprKind::Bool(_)
            | ast::ExprKind::Int(_)
            | ast::ExprKind::Unary(..)
            | ast::ExprKind::Binary(..)
            | ast::ExprKind::BoundVariable(_) => {
                let value = self.compile_expression(expr)?;
                self.ret(value)
            }
            ast::ExprKind::FreeVariable(_) => todo!(),
            ast::ExprKind::Application(left, right) => {
                let arg = self.compile_expression(right)?;
                self.push(arg)?;
                let closure = self.compile_expression(left)?;
                self.appterm(closure)
            }
            ast::ExprKind::Abstraction(_, body) => {
                let (func, block, env) = self.begin_function()?;
                self.compile_expression_tail(body)?;
                self.end_function(block, env);
                self.grab(func)
            }
            ast::ExprKind::LetIn(is_rec, _, expr, body) => todo!(),
            ast::ExprKind::IfElse(condition, consequence, alternative) => todo!(),
        }
    }

    fn compile_primitive(&mut self, expr: &ast::Expr) -> Result<IntValue<'ctx>> {
        match &expr.kind {
            ast::ExprKind::Unit => self.box_const(0),
            ast::ExprKind::Bool(b) => self.box_const(i64::from(*b)),
            ast::ExprKind::Int(n) => self.box_const(*n),
            ast::ExprKind::Unary(op, expr) => {
                let value = self.compile_expression(expr)?;
                let value = self.unbox_int(value)?;
                let result = match op {
                    ast::UnaryOp::Negate => self.builder.build_int_neg(value, "neg")?,
                    ast::UnaryOp::BitNot => self.builder.build_not(value, "bitnot")?,
                    ast::UnaryOp::Not => {
                        self.compare(IntPredicate::EQ, value, self.word.const_zero())?
                    }
                };
                self.box_int(result)
            }
            ast::ExprKind::Binary(op, left, right) => {
                let l = self.compile_expression(left)?;
                let l = self.unbox_int(l)?;
                let r = self.compile_expression(right)?;
                let r = self.unbox_int(r)?;
                let b = &self.builder;
                let result = match op {
                    ast::BinaryOp::Add => b.build_int_add(l, r, "add")?,
                    ast::BinaryOp::Sub => b.build_int_sub(l, r, "sub")?,
                    ast::BinaryOp::Mul => b.build_int_mul(l, r, "mul")?,
                    ast::BinaryOp::Div => b.build_int_signed_div(l, r, "div")?,
                    ast::BinaryOp::Lt => self.compare(IntPredicate::SLT, l, r)?,
                    ast::BinaryOp::Gt => self.compare(IntPredicate::SGT, l, r)?,
                    ast::BinaryOp::Lte => self.compare(IntPredicate::SLE, l, r)?,
                    ast::BinaryOp::Gte => self.compare(IntPredicate::SGE, l, r)?,
                    ast::BinaryOp::Eq => self.compare(IntPredicate::EQ, l, r)?,
                    ast::BinaryOp::BitAnd | ast::BinaryOp::And => b.build_and(l, r, "and")?,
                    ast::BinaryOp::BitOr | ast::BinaryOp::Or => b.build_or(l, r, "or")?,
                };
                self.box_int(result)
            }
            _ => unreachable!("not a primitive: {expr}"),
        }
    }

    fn compare(
        &self,
        predicate: IntPredicate,
        l: IntValue<'ctx>,
        r: IntValue<'ctx>,
    ) -> Result<IntValue<'ctx>> {
        let bit = self.builder.build_int_compare(predicate, l, r, "cmp")?;
        self.builder.build_int_z_extend(bit, self.word, "bool")
    }

    fn box_const(&self, n: i64) -> Result<IntValue<'ctx>> {
        self.box_int(self.word.const_int(n.cast_unsigned(), false))
    }

    fn box_int(&self, n: IntValue<'ctx>) -> Result<IntValue<'ctx>> {
        let one = self.word.const_int(1, false);
        let shifted = self.builder.build_left_shift(n, one, "shl")?;
        self.builder.build_or(shifted, one, "boxed")
    }

    fn unbox_int(&self, v: IntValue<'ctx>) -> Result<IntValue<'ctx>> {
        let one = self.word.const_int(1, false);
        self.builder.build_right_shift(v, one, true, "unboxed")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{elaborate::surface_to_core, interpret::interpret, parser::Parser};
    use inkwell::{
        OptimizationLevel,
        targets::{InitializationConfig, Target},
    };

    fn run(source: &str) -> i64 {
        // validate and run inkwell JIT
        let program = surface_to_core(Parser::parse(source).unwrap());
        let context = Context::create();
        let mut compiler = Compiler::new(&context, "test").unwrap();
        compiler.compile(&program).unwrap();
        if let Err(e) = compiler.module.verify() {
            panic!("invalid IR for {source:?}: {e}\n{}", compiler.ir());
        }

        Target::initialize_native(&InitializationConfig::default()).unwrap();
        let engine = compiler
            .module
            .create_jit_execution_engine(OptimizationLevel::None)
            .unwrap();
        unsafe {
            engine
                .get_function::<unsafe extern "C" fn() -> i64>("main")
                .unwrap()
                .call()
        }
    }

    fn agrees(source: &str) {
        // compare to reference interpreter
        let program = surface_to_core(Parser::parse(source).unwrap());
        let expected = interpret(&program).to_string();
        assert_eq!(run(source).to_string(), expected, "{source}");
    }

    #[test]
    fn compiles_expressions() {
        agrees("54321");
        agrees("-54321");
        agrees("1 + 2 * 3 - 4");
        agrees("(fun x => x) (1 + 2)");
        agrees("(fun x => x + 2) 1");
        agrees("(fun x => x) (fun x => fun y => x) 1 2");
        agrees("(fun x => fun y => x) (fun x => x) (fun x => x) 5");
        agrees("(fun x => fun y => x) (1 + 2) 99");
        agrees("(fun twice => twice (fun n => n * 3) 5) (fun f => fun x => f (f x))");
        agrees(
            "(fun x => fun y => fun z => x z (y z)) (fun x => fun _ => x) (fun x => fun _ => x) 7",
        );
    }
}
