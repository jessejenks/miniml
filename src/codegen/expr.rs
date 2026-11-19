use std::collections::HashMap;

use inkwell::{IntPredicate, basic_block::BasicBlock, module::Linkage, values::IntValue};

use super::{Compiler, Result};
use crate::{
    ast,
    codegen::Tag,
    patternmatch::{Arm, Branch, MatchVar, Tree},
};

impl<'ctx> Compiler<'ctx> {
    pub(super) fn compile_expression(&mut self, expr: &ast::Expr) -> Result<IntValue<'ctx>> {
        match &expr.kind {
            ast::ExprKind::Literal(_) | ast::ExprKind::Unary(..) | ast::ExprKind::Binary(..) => {
                self.compile_primitive(expr)
            }
            ast::ExprKind::BoundVariable(n) => self.access(*n),
            ast::ExprKind::FreeVariable(name) => self.free_variable(name, expr.span),
            ast::ExprKind::Application(left, right) => {
                self.pushmark()?;
                let arg = self.compile_expression(right)?;
                self.push(arg)?;
                let mut spine = left;
                while let ast::ExprKind::Application(left, right) = &spine.kind {
                    let arg = self.compile_expression(right)?;
                    self.push(arg)?;
                    spine = left;
                }
                let closure = self.compile_expression(spine)?;
                self.apply(closure)
            }
            ast::ExprKind::Abstraction(_, body) => {
                let (func, block, env) = self.begin_function()?;
                self.compile_expression_tail(body)?;
                self.end_function(block, env);
                self.cur(func)
            }
            ast::ExprKind::LetIn(false, pattern, expr, body) => {
                let value = self.compile_expression(expr)?;
                let count = self.bind_pattern(pattern, value)?;
                let result = self.compile_expression(body)?;
                for _ in 0..count {
                    self.end_let()?;
                }
                Ok(result)
            }
            ast::ExprKind::LetIn(true, _, expr, body) => {
                self.dummy()?;
                let value = self.compile_expression(expr)?;
                self.update(value)?;
                let result = self.compile_expression(body)?;
                self.end_let()?;
                Ok(result)
            }
            ast::ExprKind::IfElse(condition, consequence, alternative) => {
                let condition = self.compile_expression(condition)?;
                let (then_block, else_block) = self.branch_on_bool(condition, "if")?;

                self.builder.position_at_end(then_block);
                let then_value = self.compile_expression(consequence)?;
                let then_end = self.insert_block();

                self.builder.position_at_end(else_block);
                let else_value = self.compile_expression(alternative)?;
                let else_end = self.insert_block();

                self.merge("if", &[(then_value, then_end), (else_value, else_end)])
            }
            ast::ExprKind::Tuple(elements) => self.compile_tuple(elements),
            ast::ExprKind::Match(scrutinee, arms) => {
                let arm_blocks = self.compile_match(expr.span, scrutinee, arms)?;
                let mut results = Vec::new();
                for ((_, body), entry) in arms.iter().zip(arm_blocks) {
                    if let Some((block, count)) = entry {
                        self.builder.position_at_end(block);
                        let value = self.compile_expression(body)?;
                        for _ in 0..count {
                            self.end_let()?;
                        }
                        results.push((value, self.insert_block()));
                    }
                }
                self.merge("match", &results)
            }
        }
    }

    fn compile_expression_tail(&mut self, expr: &ast::Expr) -> Result<()> {
        match &expr.kind {
            ast::ExprKind::Literal(_)
            | ast::ExprKind::Unary(..)
            | ast::ExprKind::Binary(..)
            | ast::ExprKind::BoundVariable(_)
            | ast::ExprKind::FreeVariable(_)
            | ast::ExprKind::Tuple(_) => {
                let value = self.compile_expression(expr)?;
                self.ret(value)
            }
            ast::ExprKind::Application(left, right) => {
                let arg = self.compile_expression(right)?;
                self.push(arg)?;
                let mut spine = left;
                while let ast::ExprKind::Application(left, right) = &spine.kind {
                    let arg = self.compile_expression(right)?;
                    self.push(arg)?;
                    spine = left;
                }
                let closure = self.compile_expression(spine)?;
                self.appterm(closure)
            }
            ast::ExprKind::Abstraction(_, body) => {
                let (func, block, env) = self.begin_function()?;
                self.compile_expression_tail(body)?;
                self.end_function(block, env);
                self.grab(func)
            }
            ast::ExprKind::LetIn(false, pattern, expr, body) => {
                let value = self.compile_expression(expr)?;
                self.bind_pattern(pattern, value)?;
                self.compile_expression_tail(body)
            }
            ast::ExprKind::LetIn(true, _, expr, body) => {
                self.dummy()?;
                let value = self.compile_expression(expr)?;
                self.update(value)?;
                self.compile_expression_tail(body)
            }
            // no merge block for tail
            ast::ExprKind::IfElse(condition, consequence, alternative) => {
                let condition = self.compile_expression(condition)?;
                let (then_block, else_block) = self.branch_on_bool(condition, "if")?;
                self.builder.position_at_end(then_block);
                self.compile_expression_tail(consequence)?;
                self.builder.position_at_end(else_block);
                self.compile_expression_tail(alternative)
            }
            ast::ExprKind::Match(scrutinee, arms) => {
                let arm_blocks = self.compile_match(expr.span, scrutinee, arms)?;
                for ((_, body), entry) in arms.iter().zip(arm_blocks) {
                    if let Some((block, _)) = entry {
                        self.builder.position_at_end(block);
                        self.compile_expression_tail(body)?;
                    }
                }
                Ok(())
            }
        }
    }

    /// Returns each arm's entry block and binder count, `None` for arms no leaf reaches
    fn compile_match(
        &mut self,
        span: ast::Span,
        scrutinee: &ast::Expr,
        arms: &[(ast::Pattern, ast::Expr)],
    ) -> Result<Vec<Option<(BasicBlock<'ctx>, usize)>>> {
        let tree = self.trees.remove(&span).unwrap();
        let value = self.compile_expression(scrutinee)?;
        let mut vars = HashMap::from([(MatchVar(0), value)]);
        let mut arm_blocks = vec![None; arms.len()];
        self.compile_tree(&tree, &mut vars, &mut arm_blocks)?;
        Ok(arm_blocks)
    }

    fn compile_tree(
        &mut self,
        tree: &Tree,
        vars: &mut HashMap<MatchVar, IntValue<'ctx>>,
        arm_blocks: &mut [Option<(BasicBlock<'ctx>, usize)>],
    ) -> Result<()> {
        match tree {
            Tree::Err => unreachable!("compiled with fail_early"),
            Tree::Leaf(Arm(arm), bound) => {
                for var in bound {
                    self.let_in(vars[var])?;
                }
                let (block, _) = *arm_blocks[*arm].get_or_insert_with(|| {
                    (self.new_block(&format!("match.arm{arm}")), bound.len())
                });
                self.builder.build_unconditional_branch(block)?;
                Ok(())
            }
            Tree::Branch(var, Branch::Tuple(fields, rest)) => {
                let tuple = vars[var];
                for (i, field) in fields.iter().enumerate() {
                    let value = self.load_field(tuple, i)?;
                    vars.insert(*field, value);
                }
                self.compile_tree(rest, vars, arm_blocks)
            }
            Tree::Branch(_, Branch::Literal(_, matched, None)) => {
                self.compile_tree(matched, vars, arm_blocks)
            }
            Tree::Branch(var, Branch::Literal(lit, matched, Some(alternate))) => {
                let is_match = self.literal_equals(vars[var], lit)?;
                let matched_block = self.new_block("match.then");
                let alternate_block = self.new_block("match.else");
                self.builder
                    .build_conditional_branch(is_match, matched_block, alternate_block)?;
                self.builder.position_at_end(matched_block);
                self.compile_tree(matched, vars, arm_blocks)?;
                self.builder.position_at_end(alternate_block);
                self.compile_tree(alternate, vars, arm_blocks)
            }
        }
    }

    fn literal_equals(&self, value: IntValue<'ctx>, lit: &ast::Literal) -> Result<IntValue<'ctx>> {
        let lit_value = self.compile_literal(lit)?;
        if let ast::Literal::String(_) = lit {
            let eq = self
                .builder
                .build_direct_call(self.miniml_equal, &[value.into(), lit_value.into()], "eq")?
                .try_as_basic_value()
                .expect_basic("miniml_equal returns a value")
                .into_int_value();
            let zero = self.word.const_zero();
            Ok(self
                .builder
                .build_int_compare(IntPredicate::NE, eq, zero, "lit.eq")?)
        } else {
            let eq = IntPredicate::EQ;
            Ok(self
                .builder
                .build_int_compare(eq, value, lit_value, "lit.eq")?)
        }
    }

    fn new_block(&self, name: &str) -> BasicBlock<'ctx> {
        let function = self
            .insert_block()
            .get_parent()
            .expect("builder is inside a function");
        self.context.append_basic_block(function, name)
    }

    fn load_field(&self, tuple: IntValue<'ctx>, i: usize) -> Result<IntValue<'ctx>> {
        let ptr = self
            .builder
            .build_int_to_ptr(tuple, self.ptr_type, "tuple")?;
        let field = unsafe {
            self.builder.build_gep(
                self.word,
                ptr,
                &[self.word.const_int(i as u64, false)],
                "tuple.field",
            )?
        };
        Ok(self
            .builder
            .build_load(self.word, field, "field")?
            .into_int_value())
    }

    fn bind_pattern(&mut self, pattern: &ast::Pattern, value: IntValue<'ctx>) -> Result<usize> {
        match &pattern.kind {
            ast::PatternKind::Literal(_) => Ok(0),
            ast::PatternKind::Wild | ast::PatternKind::Variable => {
                self.let_in(value)?;
                Ok(1)
            }
            ast::PatternKind::Tuple(patterns) => {
                let mut count = 0;
                for (i, pattern) in patterns.iter().enumerate() {
                    let field = self.load_field(value, i)?;
                    count += self.bind_pattern(pattern, field)?;
                }
                Ok(count)
            }
        }
    }

    fn compile_literal(&self, lit: &ast::Literal) -> Result<IntValue<'ctx>> {
        match lit {
            ast::Literal::Unit => self.box_const(0),
            ast::Literal::Bool(b) => self.box_const(i64::from(*b)),
            ast::Literal::Char(c) => self.box_const(*c as i64),
            ast::Literal::Int(n) => self.box_const(*n),
            ast::Literal::String(s) => self.compile_string(s),
        }
    }

    fn compile_primitive(&mut self, expr: &ast::Expr) -> Result<IntValue<'ctx>> {
        match &expr.kind {
            ast::ExprKind::Literal(lit) => self.compile_literal(lit),
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
            ast::ExprKind::Binary(ast::BinaryOp::Concat, left, right) => {
                let l = self.compile_expression(left)?;
                let r = self.compile_expression(right)?;
                Ok(self
                    .builder
                    .build_direct_call(self.miniml_string_concat, &[l.into(), r.into()], "concat")?
                    .try_as_basic_value()
                    .expect_basic("miniml_string_concat returns a value")
                    .into_int_value())
            }
            ast::ExprKind::Binary(ast::BinaryOp::Eq, left, right) => {
                let l = self.compile_expression(left)?;
                let r = self.compile_expression(right)?;
                let eq = if Self::is_immediate(left) || Self::is_immediate(right) {
                    self.compare(IntPredicate::EQ, l, r)?
                } else {
                    self.builder
                        .build_direct_call(self.miniml_equal, &[l.into(), r.into()], "eq")?
                        .try_as_basic_value()
                        .expect_basic("miniml_equal returns a value")
                        .into_int_value()
                };
                self.box_int(eq)
            }
            ast::ExprKind::Binary(op @ (ast::BinaryOp::And | ast::BinaryOp::Or), left, right) => {
                self.compile_short_circuit(*op, left, right)
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
                    ast::BinaryOp::BitAnd => b.build_and(l, r, "and")?,
                    ast::BinaryOp::BitOr => b.build_or(l, r, "or")?,
                    ast::BinaryOp::And | ast::BinaryOp::Or => unreachable!("short-circuits"),
                    ast::BinaryOp::Eq => unreachable!("equality comparison"),
                    ast::BinaryOp::Concat => unreachable!("string concatenation"),
                };
                self.box_int(result)
            }
            _ => unreachable!("not a primitive: {expr}"),
        }
    }

    fn compile_string(&self, s: &String) -> Result<IntValue<'ctx>> {
        // OCaml / Damien Doligez string encoding
        // jessejenks.github.io/blog/ocaml-strings
        let len = s.len();
        let words = len / 8 + 1;
        let mut bytes = s.as_bytes().to_vec();
        // null padding
        bytes.resize(8 * words - 1, 0);
        // last byte
        let r = 8 * words - len;
        bytes.push(u8::try_from(r - 1).unwrap());
        let header = self
            .word
            .const_int(((words as u64) << 10) | (Tag::String as u64), false);
        let data = self.context.const_string(&bytes, false);
        let value = self
            .context
            .const_struct(&[header.into(), data.into()], false);

        let global = self.module.add_global(value.get_type(), None, "str");
        global.set_initializer(&value);
        global.set_constant(true);
        global.set_linkage(Linkage::Private);
        global.set_alignment(8);

        // point to data, not header
        let ptr = self.builder.build_struct_gep(
            value.get_type(),
            global.as_pointer_value(),
            1,
            "str.ptr",
        )?;
        Ok(self.builder.build_ptr_to_int(ptr, self.word, "str.word")?)
    }

    fn compile_tuple(&mut self, elements: &[ast::Expr]) -> Result<IntValue<'ctx>> {
        let values = elements
            .iter()
            .map(|e| self.compile_expression(e))
            .collect::<Result<Vec<_>>>()?;

        let words = elements.len();
        let tuple = self
            .builder
            .build_direct_call(
                self.miniml_alloc,
                &[
                    self.word.const_int(words as u64, false).into(),
                    self.word.const_int(Tag::Tuple as u64, false).into(),
                ],
                "tuple",
            )?
            .try_as_basic_value()
            .expect_basic("miniml_alloc returns a value")
            .into_pointer_value();

        for (i, value) in values.iter().enumerate() {
            let field = unsafe {
                self.builder.build_gep(
                    self.word,
                    tuple,
                    &[self.word.const_int(i as u64, false)],
                    "tuple.field",
                )?
            };
            self.builder.build_store(field, *value)?;
        }

        Ok(self
            .builder
            .build_ptr_to_int(tuple, self.word, "tuple.word")?)
    }

    fn compile_short_circuit(
        &mut self,
        op: ast::BinaryOp,
        left: &ast::Expr,
        right: &ast::Expr,
    ) -> Result<IntValue<'ctx>> {
        let l = self.compile_expression(left)?;
        let name = if op == ast::BinaryOp::And {
            "and"
        } else {
            "or"
        };
        let (then_block, else_block) = self.branch_on_bool(l, name)?;
        let (right_block, short_block) = if op == ast::BinaryOp::And {
            (then_block, else_block)
        } else {
            (else_block, then_block)
        };

        self.builder.position_at_end(right_block);
        let r = self.compile_expression(right)?;
        let right_end = self.insert_block();

        self.merge(name, &[(l, short_block), (r, right_end)])
    }

    fn compare(
        &self,
        predicate: IntPredicate,
        l: IntValue<'ctx>,
        r: IntValue<'ctx>,
    ) -> Result<IntValue<'ctx>> {
        let bit = self.builder.build_int_compare(predicate, l, r, "cmp")?;
        Ok(self.builder.build_int_z_extend(bit, self.word, "bool")?)
    }

    pub(super) fn box_const(&self, n: i64) -> Result<IntValue<'ctx>> {
        self.box_int(self.word.const_int(n.cast_unsigned(), false))
    }

    fn box_int(&self, n: IntValue<'ctx>) -> Result<IntValue<'ctx>> {
        let one = self.word.const_int(1, false);
        let shifted = self.builder.build_left_shift(n, one, "shl")?;
        Ok(self.builder.build_or(shifted, one, "boxed")?)
    }

    pub(super) fn unbox_int(&self, v: IntValue<'ctx>) -> Result<IntValue<'ctx>> {
        let one = self.word.const_int(1, false);
        Ok(self.builder.build_right_shift(v, one, true, "unboxed")?)
    }

    fn is_immediate(e: &ast::Expr) -> bool {
        matches!(
            e.kind,
            ast::ExprKind::Literal(
                ast::Literal::Unit
                    | ast::Literal::Bool(_)
                    | ast::Literal::Char(_)
                    | ast::Literal::Int(_)
            )
        )
    }
}
