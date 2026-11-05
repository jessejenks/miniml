use inkwell::{
    IntPredicate,
    builder::{Builder, BuilderError},
    context::Context,
    module::Module,
    types::IntType,
    values::IntValue,
};

use crate::ast;

type Result<T> = std::result::Result<T, BuilderError>;

pub struct Compiler<'ctx> {
    context: &'ctx Context,
    module: Module<'ctx>,
    builder: Builder<'ctx>,
    word: IntType<'ctx>,
}

impl<'ctx> Compiler<'ctx> {
    pub fn new(context: &'ctx Context) -> Self {
        Self {
            context,
            module: context.create_module("miniml"),
            builder: context.create_builder(),
            word: context.i64_type(),
        }
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
            ast::ExprKind::BoundVariable(_) => todo!(),
            ast::ExprKind::FreeVariable(_) => todo!(),
            ast::ExprKind::Application(left, right) => todo!(),
            ast::ExprKind::Abstraction(_, body) => todo!(),
            ast::ExprKind::LetIn(is_rec, _, expr, body) => todo!(),
            ast::ExprKind::IfElse(condition, consequence, alternative) => todo!(),
        }
    }

    fn compile_expression_tail(&mut self, expr: &ast::Expr) -> Result<IntValue<'ctx>> {
        match &expr.kind {
            ast::ExprKind::Unit
            | ast::ExprKind::Bool(_)
            | ast::ExprKind::Int(_)
            | ast::ExprKind::Unary(..)
            | ast::ExprKind::Binary(..) => self.compile_primitive(expr),
            ast::ExprKind::BoundVariable(_) => todo!(),
            ast::ExprKind::FreeVariable(_) => todo!(),
            ast::ExprKind::Application(left, right) => todo!(),
            ast::ExprKind::Abstraction(_, body) => todo!(),
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
