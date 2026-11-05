use std::{cmp, collections::HashMap, fmt::Display, rc::Rc};

use miette::{Diagnostic, SourceSpan};
use thiserror::Error;

use crate::ast;

#[derive(Debug, Clone, PartialEq)]
pub enum TypeExpr {
    Unit,
    Bool,
    Int,
    Var(i64),
    UnifVar(u32),
    Arrow(Rc<TypeExpr>, Rc<TypeExpr>),
}

impl TypeExpr {
    fn greek_letter(n: u32) -> (char, u32) {
        let letter = char::from_u32(u32::from('α') + n % 25).expect("α..ω are valid chars");
        (letter, n / 25)
    }

    fn create_existential_variable_name(id: u32) -> String {
        const COMBINING_CIRCUMFLEX_ACCENT: char = '\u{0302}';
        match Self::greek_letter(id) {
            (letter, 0) => format!("{letter}{COMBINING_CIRCUMFLEX_ACCENT}"),
            (letter, c) => format!("{letter}{COMBINING_CIRCUMFLEX_ACCENT}{c}"),
        }
    }

    fn create_type_variable_name(level: i64) -> String {
        let level = u32::try_from(level).expect("type variable levels are non-negative");
        match Self::greek_letter(level) {
            (letter, 0) => letter.to_string(),
            (letter, c) => format!("{letter}{c}"),
        }
    }

    pub fn max_type_variable_level(tp: &TypeExpr) -> i64 {
        match tp {
            TypeExpr::Unit | TypeExpr::Bool | TypeExpr::Int | TypeExpr::UnifVar(_) => -1,
            TypeExpr::Var(level) => *level,
            TypeExpr::Arrow(src, tgt) => cmp::max(
                Self::max_type_variable_level(src),
                Self::max_type_variable_level(tgt),
            ),
        }
    }
}

impl Display for TypeExpr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TypeExpr::Unit => write!(f, "Unit"),
            TypeExpr::Bool => write!(f, "Bool"),
            TypeExpr::Int => write!(f, "Int"),
            TypeExpr::Var(level) => write!(f, "{}", Self::create_type_variable_name(*level)),
            TypeExpr::UnifVar(id) => {
                write!(f, "{}", Self::create_existential_variable_name(*id))
            }
            TypeExpr::Arrow(src, tgt) => {
                if matches!(src.as_ref(), TypeExpr::Arrow(_, _)) {
                    write!(f, "({src}) -> {tgt}")
                } else {
                    write!(f, "{src} -> {tgt}")
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Scheme {
    arity: u32,
    tp: TypeExpr,
}

impl Display for Scheme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.arity == 0 {
            self.tp.fmt(f)
        } else {
            write!(f, "(")?;
            for level in 0..=TypeExpr::max_type_variable_level(&self.tp) {
                write!(f, "∀{}.", TypeExpr::create_type_variable_name(level))?;
            }
            write!(f, "{})", self.tp)
        }
    }
}

#[derive(Debug, Error, Diagnostic)]
pub enum TypeError {
    #[error("unbound variable `{name}`")]
    #[diagnostic(code(typecheck::unbound_variable), help("the only builtin is `print`"))]
    UnboundVariable {
        name: String,
        #[label("not defined")]
        span: SourceSpan,
    },
    #[error("variable #{index} is not in scope")]
    #[diagnostic(code(typecheck::unbound_index))]
    UnboundIndex {
        index: i64,
        #[label("not in scope")]
        span: SourceSpan,
    },
    #[error("could not unify {left} with {right}")]
    #[diagnostic(code(typecheck::unification))]
    Unification {
        left: String,
        right: String,
        #[label("this term")]
        span: SourceSpan,
    },
    #[error("occurs check failed for {tp}")]
    #[diagnostic(code(typecheck::occurs_check))]
    OccursCheck {
        tp: String,
        #[label("this term")]
        span: SourceSpan,
    },
}

type Result<T> = std::result::Result<T, TypeError>;

pub struct TypeChecker {
    env: HashMap<String, Scheme>,
    context: Vec<Scheme>,
    substitution: Vec<Option<TypeExpr>>,
    levels: Vec<i64>,
    current_level: i64,
}

impl TypeChecker {
    pub fn new() -> Self {
        Self {
            env: HashMap::new(),
            context: Vec::new(),
            substitution: Vec::new(),
            levels: Vec::new(),
            current_level: 0,
        }
    }

    pub fn infer(&mut self, expr: &ast::Expr) -> Result<Scheme> {
        let tp = self.algorithm_j(expr)?;
        let resolved = self.resolve(tp);
        let scheme = self.generalize_at_level(resolved, -1);
        Ok(scheme)
    }

    fn algorithm_j(&mut self, expr: &ast::Expr) -> Result<TypeExpr> {
        match &expr.kind {
            ast::ExprKind::Unit => Ok(TypeExpr::Unit),
            ast::ExprKind::Bool(_) => Ok(TypeExpr::Bool),
            ast::ExprKind::Int(_) => Ok(TypeExpr::Int),
            ast::ExprKind::BoundVariable(index) => self.rule_var(expr.span, *index),
            ast::ExprKind::FreeVariable(name) => self.rule_free_var(expr.span, name.clone()),
            ast::ExprKind::Application(left, right) => self.rule_app(expr.span, left, right),
            ast::ExprKind::Abstraction(_, body) => self.rule_abs(body),
            ast::ExprKind::LetIn(false, _, e, body) => self.rule_let(e, body),
            ast::ExprKind::LetIn(true, _, e, body) => self.rule_let_rec(expr.span, e, body),
            ast::ExprKind::IfElse(condition, consequence, alternative) => {
                self.rule_if(expr.span, condition, consequence, alternative)
            }
            ast::ExprKind::Unary(op, e) => self.rule_unary(expr.span, *op, e),
            ast::ExprKind::Binary(op, left, right) => self.rule_binary(expr.span, *op, left, right),
        }
    }

    fn rule_var(&mut self, span: ast::Span, index: i64) -> Result<TypeExpr> {
        let scheme = usize::try_from(index)
            .ok()
            .and_then(|i| self.context.len().checked_sub(i + 1))
            .map(|position| self.context[position].clone())
            .ok_or(TypeError::UnboundIndex {
                index,
                span: span.into(),
            })?;
        Ok(self.instantiate(&scheme))
    }

    fn rule_free_var(&mut self, span: ast::Span, name: String) -> Result<TypeExpr> {
        match self.env.get(&name).cloned() {
            Some(scheme) => Ok(self.instantiate(&scheme)),
            None => Err(TypeError::UnboundVariable {
                name,
                span: span.into(),
            }),
        }
    }

    fn rule_app(
        &mut self,
        span: ast::Span,
        left: &ast::Expr,
        right: &ast::Expr,
    ) -> Result<TypeExpr> {
        let t1 = self.algorithm_j(left)?;
        let t2 = self.algorithm_j(right)?;
        let t_prime = self.new_unification_variable();
        self.unify(
            span,
            t1,
            TypeExpr::Arrow(Rc::new(t2), Rc::new(t_prime.clone())),
        )?;
        Ok(t_prime)
    }

    fn rule_abs(&mut self, body: &ast::Expr) -> Result<TypeExpr> {
        let t = self.new_unification_variable();
        self.context.push(Self::as_monotype(t.clone()));
        let t_prime = self.algorithm_j(body)?;
        self.context.pop();
        Ok(TypeExpr::Arrow(Rc::new(t), Rc::new(t_prime)))
    }

    fn rule_let(&mut self, expr: &ast::Expr, body: &ast::Expr) -> Result<TypeExpr> {
        self.current_level += 1;
        let t_prime = self.algorithm_j(expr)?;
        self.current_level -= 1;
        let t_prime_gen = self.generalize(t_prime);
        self.context.push(t_prime_gen);
        let t = self.algorithm_j(body)?;
        self.context.pop();
        Ok(t)
    }

    fn rule_let_rec(
        &mut self,
        span: ast::Span,
        expr: &ast::Expr,
        body: &ast::Expr,
    ) -> Result<TypeExpr> {
        self.current_level += 1;
        let t_prime = self.new_unification_variable();
        self.context.push(Self::as_monotype(t_prime.clone()));
        let t1 = self.algorithm_j(expr)?;
        self.context.pop();
        self.unify(span, t_prime.clone(), t1)?;
        self.current_level -= 1;
        let t_prime_gen = self.generalize(t_prime);
        self.context.push(t_prime_gen);
        let t = self.algorithm_j(body)?;
        self.context.pop();
        Ok(t)
    }

    fn rule_if(
        &mut self,
        span: ast::Span,
        condition: &ast::Expr,
        consequence: &ast::Expr,
        alternative: &ast::Expr,
    ) -> Result<TypeExpr> {
        let condition_tp = self.algorithm_j(condition)?;
        self.unify(span, condition_tp, TypeExpr::Bool)?;
        let cons_tp = self.algorithm_j(consequence)?;
        let alt_tp = self.algorithm_j(alternative)?;
        self.unify(span, cons_tp.clone(), alt_tp)?;
        Ok(cons_tp)
    }

    fn rule_unary(
        &mut self,
        span: ast::Span,
        op: ast::UnaryOp,
        expr: &ast::Expr,
    ) -> Result<TypeExpr> {
        let tp = self.algorithm_j(expr)?;
        match op {
            ast::UnaryOp::Negate | ast::UnaryOp::BitNot => {
                self.unify(span, tp, TypeExpr::Int)?;
                Ok(TypeExpr::Int)
            }
            ast::UnaryOp::Not => {
                self.unify(span, tp, TypeExpr::Bool)?;
                Ok(TypeExpr::Bool)
            }
        }
    }

    fn rule_binary(
        &mut self,
        span: ast::Span,
        op: ast::BinaryOp,
        left: &ast::Expr,
        right: &ast::Expr,
    ) -> Result<TypeExpr> {
        let l_tp = self.algorithm_j(left)?;
        let r_tp = self.algorithm_j(right)?;
        match op {
            ast::BinaryOp::Add
            | ast::BinaryOp::Sub
            | ast::BinaryOp::Mul
            | ast::BinaryOp::Div
            | ast::BinaryOp::BitAnd
            | ast::BinaryOp::BitOr => {
                self.unify(span, l_tp, TypeExpr::Int)?;
                self.unify(span, r_tp, TypeExpr::Int)?;
                Ok(TypeExpr::Int)
            }
            ast::BinaryOp::Lt | ast::BinaryOp::Gt | ast::BinaryOp::Lte | ast::BinaryOp::Gte => {
                self.unify(span, l_tp, TypeExpr::Int)?;
                self.unify(span, r_tp, TypeExpr::Int)?;
                Ok(TypeExpr::Bool)
            }
            ast::BinaryOp::And | ast::BinaryOp::Or => {
                self.unify(span, l_tp, TypeExpr::Bool)?;
                self.unify(span, r_tp, TypeExpr::Bool)?;
                Ok(TypeExpr::Bool)
            }
            ast::BinaryOp::Eq => todo!(),
        }
    }

    fn as_monotype(tp: TypeExpr) -> Scheme {
        Scheme { arity: 0, tp }
    }

    fn new_unification_variable(&mut self) -> TypeExpr {
        let id = u32::try_from(self.levels.len()).unwrap();
        self.levels.push(self.current_level);
        self.substitution.push(None);
        TypeExpr::UnifVar(id)
    }

    fn unify(&mut self, span: ast::Span, left_init: TypeExpr, right_init: TypeExpr) -> Result<()> {
        let left = self.resolve(left_init);
        let right = self.resolve(right_init);
        match (left, right) {
            (TypeExpr::Unit, TypeExpr::Unit)
            | (TypeExpr::Bool, TypeExpr::Bool)
            | (TypeExpr::Int, TypeExpr::Int) => Ok(()),
            (TypeExpr::Arrow(ss, st), TypeExpr::Arrow(ts, tt)) => {
                self.unify(span, (*ss).clone(), (*ts).clone())?;
                self.unify(span, (*st).clone(), (*tt).clone())
            }
            (TypeExpr::UnifVar(l_id), TypeExpr::UnifVar(r_id)) => {
                if l_id == r_id {
                    return Ok(());
                }
                let (l_idx, r_idx) = (
                    usize::try_from(l_id).unwrap(),
                    usize::try_from(r_id).unwrap(),
                );
                if self.levels[l_idx] <= self.levels[r_idx] {
                    self.levels[r_idx] = self.levels[l_idx];
                    self.substitution[l_idx] = Some(TypeExpr::UnifVar(r_id));
                } else {
                    self.levels[l_idx] = self.levels[r_idx];
                    self.substitution[r_idx] = Some(TypeExpr::UnifVar(l_id));
                }
                Ok(())
            }
            (TypeExpr::UnifVar(id), tp) | (tp, TypeExpr::UnifVar(id)) => {
                if self.occurs(id, tp.clone()) {
                    Err(TypeError::OccursCheck {
                        tp: self.resolve(tp).to_string(),
                        span: span.into(),
                    })
                } else {
                    let idx = usize::try_from(id).unwrap();
                    let level = self.levels[idx];
                    self.lower_levels(tp.clone(), level);
                    self.substitution[idx] = Some(tp);
                    Ok(())
                }
            }
            (l, r) if l == r => Ok(()),
            (l, r) => Err(TypeError::Unification {
                left: self.resolve(l).to_string(),
                right: self.resolve(r).to_string(),
                span: span.into(),
            }),
        }
    }

    fn resolve(&mut self, tp: TypeExpr) -> TypeExpr {
        match tp {
            TypeExpr::UnifVar(mut id) => {
                let index = |id: u32| usize::try_from(id).unwrap();
                let mut path = Vec::new();
                while let Some(TypeExpr::UnifVar(next)) = &self.substitution[index(id)] {
                    path.push(id);
                    id = *next;
                }
                let result = self.substitution[index(id)]
                    .clone()
                    .unwrap_or(TypeExpr::UnifVar(id));
                // path compress
                for var in path {
                    self.substitution[index(var)] = Some(result.clone());
                }
                result
            }
            x => x,
        }
    }

    // open
    fn instantiate(&mut self, scheme: &Scheme) -> TypeExpr {
        if scheme.arity == 0 {
            return scheme.tp.clone();
        }
        let fresh: Vec<TypeExpr> = (0..scheme.arity)
            .map(|_| self.new_unification_variable())
            .collect();
        Self::instantiate_inner(&scheme.tp, &fresh)
    }

    fn instantiate_inner(tp: &TypeExpr, fresh: &[TypeExpr]) -> TypeExpr {
        match tp {
            TypeExpr::Var(level) => {
                fresh[usize::try_from(*level).expect("levels are non-negative")].clone()
            }
            TypeExpr::Arrow(src, tgt) => TypeExpr::Arrow(
                Rc::new(Self::instantiate_inner(src, fresh)),
                Rc::new(Self::instantiate_inner(tgt, fresh)),
            ),
            TypeExpr::Unit | TypeExpr::Bool | TypeExpr::Int | TypeExpr::UnifVar(_) => tp.clone(),
        }
    }

    // close
    fn generalize(&mut self, tp: TypeExpr) -> Scheme {
        self.generalize_at_level(tp, self.current_level)
    }

    fn generalize_at_level(&mut self, tp: TypeExpr, level: i64) -> Scheme {
        let mut generalized = HashMap::new();
        let tp = self.generalize_inner(tp, level, &mut generalized);
        Scheme {
            arity: u32::try_from(generalized.len()).unwrap(),
            tp,
        }
    }

    fn generalize_inner(
        &mut self,
        tp: TypeExpr,
        level: i64,
        generalized: &mut HashMap<u32, i64>,
    ) -> TypeExpr {
        match self.resolve(tp) {
            TypeExpr::UnifVar(id) => {
                let idx = usize::try_from(id).unwrap();
                if self.levels[idx] > level {
                    let next = i64::try_from(generalized.len()).unwrap();
                    TypeExpr::Var(*generalized.entry(id).or_insert(next))
                } else {
                    TypeExpr::UnifVar(id)
                }
            }
            TypeExpr::Arrow(src, tgt) => TypeExpr::Arrow(
                Rc::new(self.generalize_inner((*src).clone(), level, generalized)),
                Rc::new(self.generalize_inner((*tgt).clone(), level, generalized)),
            ),
            other => other,
        }
    }

    fn lower_levels(&mut self, tp: TypeExpr, max_level: i64) {
        match self.resolve(tp) {
            TypeExpr::UnifVar(id) => {
                let idx = usize::try_from(id).unwrap();
                let level = &mut self.levels[idx];
                *level = (*level).min(max_level);
            }
            TypeExpr::Arrow(src, tgt) => {
                self.lower_levels((*src).clone(), max_level);
                self.lower_levels((*tgt).clone(), max_level);
            }
            TypeExpr::Unit | TypeExpr::Bool | TypeExpr::Int | TypeExpr::Var(_) => {}
        }
    }

    fn occurs(&mut self, id: u32, tp: TypeExpr) -> bool {
        match self.resolve(tp) {
            TypeExpr::Unit | TypeExpr::Bool | TypeExpr::Int | TypeExpr::Var(_) => false,
            TypeExpr::UnifVar(tp_id) => id == tp_id,
            TypeExpr::Arrow(src, tgt) => {
                self.occurs(id, (*src).clone()) || self.occurs(id, (*tgt).clone())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{elaborate::surface_to_core, parser::Parser};

    fn check(source: &str) -> Result<String> {
        let program = surface_to_core(Parser::parse(source).unwrap());
        TypeChecker::new()
            .infer(&program)
            .map(|scheme| scheme.to_string())
    }

    fn arrow(left: TypeExpr, right: TypeExpr) -> TypeExpr {
        TypeExpr::Arrow(Rc::new(left), Rc::new(right))
    }

    fn bound_variable(index: i64) -> ast::Expr {
        ast::Expr {
            kind: ast::ExprKind::BoundVariable(index),
            span: (0..1).into(),
        }
    }

    #[test]
    fn infers_expressions() {
        assert_eq!(check("()").unwrap(), "Unit");
        assert_eq!(check("true").unwrap(), "Bool");
        assert_eq!(check("1").unwrap(), "Int");
        assert_eq!(check("-1 + 2 * 3").unwrap(), "Int");
        assert_eq!(check("1 < 2 && !false").unwrap(), "Bool");
        assert_eq!(check("1; true").unwrap(), "Bool");
        assert_eq!(check("fun x => x").unwrap(), "(∀α.α -> α)");
        assert_eq!(check("fun x => fun y => x").unwrap(), "(∀α.∀β.α -> β -> α)");
        assert_eq!(check("(fun x => x) 1").unwrap(), "Int");
        assert_eq!(check("fun x => x + 1").unwrap(), "Int -> Int");
        assert_eq!(
            check("fun f => fun x => f (f x)").unwrap(),
            "(∀α.(α -> α) -> α -> α)"
        );
        assert_eq!(
            check("fun x => fun y => fun z => x z (y z)").unwrap(),
            "(∀α.∀β.∀γ.(α -> β -> γ) -> (α -> β) -> α -> γ)"
        );
        assert_eq!(check("fun b => if b then 1 else 2").unwrap(), "Bool -> Int");
        assert_eq!(check("let x = 1; x + x").unwrap(), "Int");
        assert_eq!(check("let id = fun x => x; id 1; id true").unwrap(), "Bool");
        assert_eq!(
            check("let k = fun x => fun y => x; k 1").unwrap(),
            "(∀α.α -> Int)"
        );
        // y should not generalize since x is outside of let
        assert_eq!(
            check("fun x => { let y = x; y + 1; y }").unwrap(),
            "Int -> Int"
        );
        assert_eq!(
            check("let rec f = fun n => if n < 2 then 1 else n * f (n - 1); f").unwrap(),
            "Int -> Int"
        );
        assert_eq!(
            check("let rec id = fun x => x; id 1; id true").unwrap(),
            "Bool"
        );
        assert_eq!(
            check("let rec loop = fun x => loop x; loop").unwrap(),
            "(∀α.∀β.α -> β)"
        );
    }

    #[test]
    fn type_errors() {
        assert!(matches!(
            check("1 + true"),
            Err(TypeError::Unification { .. })
        ));
        assert!(matches!(check("!1"), Err(TypeError::Unification { .. })));
        assert!(matches!(check("1 2"), Err(TypeError::Unification { .. })));
        assert!(matches!(
            check("if 1 then 2 else 3"),
            Err(TypeError::Unification { .. })
        ));
        assert!(matches!(
            check("if true then 1 else false"),
            Err(TypeError::Unification { .. })
        ));
        // lambda-bound variables are monomorphic
        assert!(matches!(
            check("(fun id => { id 1; id true }) (fun x => x)"),
            Err(TypeError::Unification { .. })
        ));
        // recursive calls are monomorphic
        assert!(matches!(
            check("let rec f = fun x => { f 1; f true; x }; f"),
            Err(TypeError::Unification { .. })
        ));
        assert!(matches!(
            check("fun x => x x"),
            Err(TypeError::OccursCheck { .. })
        ));
        assert!(matches!(
            check("let rec f = fun x => f; f"),
            Err(TypeError::OccursCheck { .. })
        ));
    }

    #[test]
    fn resolves_chains() {
        let mut checker = TypeChecker::new();
        let a = checker.new_unification_variable();
        let b = checker.new_unification_variable();
        let c = checker.new_unification_variable();
        checker.substitution[0] = Some(b.clone());
        checker.substitution[1] = Some(c.clone());
        checker.substitution[2] = Some(arrow(TypeExpr::Int, TypeExpr::Bool));

        assert_eq!(checker.resolve(a).to_string(), "Int -> Bool");
        assert_eq!(checker.substitution[0], checker.substitution[2]);
        assert_eq!(checker.substitution[1], checker.substitution[2]);

        let unbound = checker.new_unification_variable();
        assert_eq!(checker.resolve(unbound.clone()), unbound);
        assert_eq!(checker.resolve(TypeExpr::Int), TypeExpr::Int);
    }

    #[test]
    fn unbound_variables() {
        assert!(matches!(
            check("foo"),
            Err(TypeError::UnboundVariable { name, .. }) if name == "foo"
        ));
        assert!(matches!(
            TypeChecker::new().algorithm_j(&bound_variable(0)),
            Err(TypeError::UnboundIndex { index: 0, .. })
        ));
        assert!(matches!(
            TypeChecker::new().algorithm_j(&bound_variable(-1)),
            Err(TypeError::UnboundIndex { index: -1, .. })
        ));
    }
}
