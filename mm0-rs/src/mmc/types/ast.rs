//! The AST, the result after parsing and name resolution.
//!
//! This is produced by the [`build_ast`](super::super::build_ast) pass,
//! and consumed by the [`ast_lower`](super::super::ast_lower) pass.
//! At this point all the renaming shenanigans in the surface syntax are gone
//! and all variables are declared only once, so we can start to apply SSA-style
//! analysis to the result. We still haven't typechecked anything, so it's
//! possible that variables are applied to the wrong arguments and so on.
//!
//! One complication where type checking affects name resolution has to do with
//! determining when variables are renamed after a mutation. Consider:
//! ```text
//! (begin
//!   {x := 1}
//!   (assert {x = 1})
//!   {{y : (&sn x)} := (& x)}
//!   {(* y) <- 2}
//!   (assert {x = 2}))
//! ```
//! It is important for us to resolve the `x` in the last line to a fresh variable
//! `x'`  distinct from the `x` on the first line, because we know that `x = 1`.
//! In the surface syntax this is explained as name shadowing - we have a new `x`
//! and references resolve to that instead, but once we have done name resolution
//! we would like these two to actually resolve to different variables. However,
//! the line responsible for the modification of `x`, `{(* y) <- 2}`, doesn't
//! mention `x` at all - it is only when we know the type of `y` that we can
//! determine that `(* y)` resolves to `x` as an lvalue.
//!
//! We could delay name resolution until type checking, but this makes things a
//! lot more complicated, and possibly also harder to reason about at the user
//! level. The current compromise is that one has to explicitly declare any
//! variable renames using `with` if they aren't syntactically obvious, so in
//! this case you would have to write `{{(* y) <- 2} with {x -> x'}}` to say that
//! `x` changes (or `{{(* y) <- 2} with x}` if the name shadowing is acceptable).

use num::BigInt;
use crate::elab::environment::{AtomId, Remap, Remapper};
use crate::elab::lisp::LispVal;
use super::{VarId, Spanned, Size, Mm0Expr, Unop, Binop, FieldName, entity::Intrinsic};

/// A "lifetime" in MMC is a variable or place from which references can be derived.
/// For example, if we `let y = &x[1]` then `y` has the type `(& x T)`. As long as
/// heap variables referring to lifetime `x` exist, `x` cannot be modified or dropped.
/// There is a special lifetime `extern` that represents inputs to the current function.
#[derive(Clone, Copy, Debug)]
pub enum Lifetime {
  /// The `extern` lifetime is the inferred lifetime for function arguments such as
  /// `fn f(x: &T)`.
  Extern,
  /// A variable lifetime `x` is the annotation on references derived from `x`
  /// (or derived from other references derived from `x`).
  Place(VarId),
  /// A lifetime that has not been inferred yet.
  Infer,
}
crate::deep_size_0!(Lifetime);

impl std::fmt::Display for Lifetime {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Lifetime::Extern => "extern".fmt(f),
      Lifetime::Place(v) => v.fmt(f),
      Lifetime::Infer => "_".fmt(f),
    }
  }
}

/// A tuple pattern, which destructures the results of assignments from functions with
/// mutiple return values, as well as explicit tuple values and structs.
pub type TuplePattern = Spanned<TuplePatternKind>;

/// A tuple pattern, which destructures the results of assignments from functions with
/// mutiple return values, as well as explicit tuple values and structs.
#[derive(Debug, DeepSizeOf)]
pub enum TuplePatternKind {
  /// A variable binding, or `_` for an ignored binding. The `bool` is true if the variable
  /// is ghost.
  Name(bool, VarId),
  /// A type ascription. The type is unparsed.
  Typed(Box<TuplePattern>, Box<Type>),
  /// A tuple, with the given arguments.
  Tuple(Box<[TuplePattern]>),
}

impl Remap for TuplePatternKind {
  type Target = Self;
  fn remap(&self, r: &mut Remapper) -> Self {
    match self {
      &TuplePatternKind::Name(b, v) => TuplePatternKind::Name(b, v),
      TuplePatternKind::Typed(pat, ty) => TuplePatternKind::Typed(pat.remap(r), ty.remap(r)),
      TuplePatternKind::Tuple(pats) => TuplePatternKind::Tuple(pats.remap(r)),
    }
  }
}

impl TuplePatternKind {
  /// Extracts the single name of this tuple pattern, or `None`
  /// if this does any tuple destructuring.
  #[must_use] pub fn as_single_name(&self) -> Option<VarId> {
    match self {
      &Self::Name(_, v) => Some(v),
      Self::Typed(pat, _) => pat.k.as_single_name(),
      Self::Tuple(_) => None
    }
  }
}

/// An argument declaration for a function.
pub type Arg = Spanned<(ArgAttr, ArgKind)>;

/// An argument declaration for a function.
#[derive(Debug, DeepSizeOf)]
pub enum ArgKind {
  /// A standard argument of the form `{x : T}`, a "lambda binder"
  Lam(TuplePatternKind),
  /// A substitution argument of the form `{{x : T} := val}`. (These are not supplied in
  /// invocations, they act as let binders in the remainder of the arguments.)
  Let(TuplePattern, Box<Expr>),
}

impl Remap for ArgKind {
  type Target = Self;
  fn remap(&self, r: &mut Remapper) -> Self {
    match self {
      ArgKind::Lam(pat) => ArgKind::Lam(pat.remap(r)),
      ArgKind::Let(pat, val) => ArgKind::Let(pat.remap(r), val.remap(r)),
    }
  }
}

impl ArgKind {
  /// Extracts the binding part of this argument.
  #[must_use] pub fn var(&self) -> &TuplePatternKind {
    match self {
      Self::Lam(pat) | Self::Let(Spanned {k: pat, ..}, _) => pat,
    }
  }
}

/// The polarity of a hypothesis binder in a match statement, which determines
/// whether it will appear positively and/or negatively. (A variable that cannot appear in
/// either polarity is a compile error.)
#[derive(Copy, Clone, Debug)]
#[repr(u8)]
pub enum PosNeg {
  /// This hypothesis appears positively, but not negatively:
  /// ```text
  /// (match e
  ///   {{{h : 1} with {e = 2}} => can use h: $ e = 1 $}
  ///   {_ => cannot use h: $ e != 1 $})
  /// ```
  Pos = 1,
  /// This hypothesis appears negatively, but not positively:
  /// ```text
  /// (match e
  ///   {{{h : 1} or 2} => cannot use h: $ e = 1 $}
  ///   {_ => can use h: $ e != 1 $})
  /// ```
  Neg = 2,
  /// This hypothesis appears both positively and negatively:
  /// ```text
  /// (match e
  ///   {{h : {1 or 2}} => can use h: $ e = 1 \/ e = 2 $}
  ///   {_ => can use h: $ ~(e = 1 \/ e = 2) $})
  /// ```
  Both = 3
}
crate::deep_size_0!(PosNeg);

impl PosNeg {
  /// Construct a `PosNeg` from positive and negative pieces.
  #[must_use] pub fn new(pos: bool, neg: bool) -> Option<PosNeg> {
    match (pos, neg) {
      (false, false) => None,
      (true, false) => Some(Self::Pos),
      (false, true) => Some(Self::Neg),
      (true, true) => Some(Self::Both),
    }
  }
  /// Does this `PosNeg` admit positive occurrences?
  #[inline] #[must_use] pub fn is_pos(self) -> bool { self as u8 & 1 != 0 }
  /// Does this `PosNeg` admit negative occurrences?
  #[inline] #[must_use] pub fn is_neg(self) -> bool { self as u8 & 2 != 0 }
}

/// A pattern, the left side of a switch statement.
pub type Pattern = Spanned<PatternKind>;

/// A pattern, the left side of a switch statement.
#[derive(Debug, DeepSizeOf)]
pub enum PatternKind {
  /// A variable binding.
  Var(VarId),
  /// A constant value.
  Const(AtomId),
  /// A numeric literal.
  Number(BigInt),
  /// A hypothesis pattern, which binds the first argument to a proof that the
  /// scrutinee satisfies the pattern argument.
  Hyped(PosNeg, VarId, Box<Pattern>),
  /// A pattern guard: Matches the inner pattern, and then if the expression returns
  /// true, this is also considered to match.
  With(Box<Pattern>, Box<Expr>),
  /// A disjunction of patterns.
  Or(Box<[Pattern]>),
}

impl Remap for PatternKind {
  type Target = Self;
  fn remap(&self, r: &mut Remapper) -> Self {
    match self {
      &PatternKind::Var(v) => PatternKind::Var(v),
      &PatternKind::Const(c) => PatternKind::Const(c.remap(r)),
      PatternKind::Number(n) => PatternKind::Number(n.clone()),
      PatternKind::Hyped(pn, v, pat) => PatternKind::Hyped(*pn, *v, pat.remap(r)),
      PatternKind::With(pat, e) => PatternKind::With(pat.remap(r), e.remap(r)),
      PatternKind::Or(pat) => PatternKind::Or(pat.remap(r)),
    }
  }
}

/// A type expression.
pub type Type = Spanned<TypeKind>;

/// A type variable index.
pub type TyVarId = u32;

/// A type, which classifies regular variables (not type variables, not hypotheses).
#[derive(Debug, DeepSizeOf)]
pub enum TypeKind {
  /// `()` is the type with one element; `sizeof () = 0`.
  Unit,
  /// `bool` is the type of booleans, that is, bytes which are 0 or 1; `sizeof bool = 1`.
  Bool,
  /// A type variable.
  Var(TyVarId),
  /// `i(8*N)` is the type of N byte signed integers `sizeof i(8*N) = N`.
  Int(Size),
  /// `u(8*N)` is the type of N byte unsigned integers; `sizeof u(8*N) = N`.
  UInt(Size),
  /// The type `[T; n]` is an array of `n` elements of type `T`;
  /// `sizeof [T; n] = sizeof T * n`.
  Array(Box<Type>, Box<Expr>),
  /// `own T` is a type of owned pointers. The typehood predicate is
  /// `x :> own T` iff `E. v (x |-> v) * v :> T`.
  Own(Box<Type>),
  /// `(ref T)` is a type of borrowed values. This type is elaborated to
  /// `(ref a T)` where `a` is a lifetime; this is handled a bit differently than rust
  /// (see [`Lifetime`]).
  Ref(Option<Box<Spanned<Lifetime>>>, Box<Type>),
  /// `(& T)` is a type of borrowed pointers. This type is elaborated to
  /// `(& a T)` where `a` is a lifetime; this is handled a bit differently than rust
  /// (see [`Lifetime`]).
  Shr(Option<Box<Spanned<Lifetime>>>, Box<Type>),
  /// `&sn x` is the type of pointers to the place `x` (a variable or indexing expression).
  RefSn(Box<Expr>),
  /// `(A, B, C)` is a tuple type with elements `A, B, C`;
  /// `sizeof (list A B C) = sizeof A + sizeof B + sizeof C`.
  List(Box<[Type]>),
  /// `(sn {a : T})` the type of values of type `T` that are equal to `a`.
  /// This is useful for asserting that a computationally relevant value can be
  /// expressed in terms of computationally irrelevant parts.
  Sn(Box<Expr>),
  /// `{x : A, y : B, z : C}` is the dependent version of `list`;
  /// it is a tuple type with elements `A, B, C`, but the types `A, B, C` can
  /// themselves refer to `x, y, z`.
  /// `sizeof {x : A, _ : B x} = sizeof A + max_x (sizeof (B x))`.
  ///
  /// The top level declaration `(struct foo {x : A} {y : B})` desugars to
  /// `(typedef foo {x : A, y : B})`.
  Struct(Box<[Arg]>),
  /// `(and A B C)` is an intersection type of `A, B, C`;
  /// `sizeof (and A B C) = max (sizeof A, sizeof B, sizeof C)`, and
  /// the typehood predicate is `x :> (and A B C)` iff
  /// `x :> A /\ x :> B /\ x :> C`. (Note that this is regular conjunction,
  /// not separating conjunction.)
  And(Box<[Type]>),
  /// `(or A B C)` is an undiscriminated anonymous union of types `A, B, C`.
  /// `sizeof (or A B C) = max (sizeof A, sizeof B, sizeof C)`, and
  /// the typehood predicate is `x :> (or A B C)` iff
  /// `x :> A \/ x :> B \/ x :> C`.
  Or(Box<[Type]>),
  /// `(or A B C)` is an undiscriminated anonymous union of types `A, B, C`.
  /// `sizeof (or A B C) = max (sizeof A, sizeof B, sizeof C)`, and
  /// the typehood predicate is `x :> (or A B C)` iff
  /// `x :> A \/ x :> B \/ x :> C`.
  If(Box<Expr>, Box<Type>, Box<Type>),
  /// A switch (pattern match) statement, given the initial expression and a list of match arms.
  Match(Box<Expr>, Box<[(Pattern, Type)]>),
  /// `(ghost A)` is a computationally irrelevant version of `A`, which means
  /// that the logical storage of `(ghost A)` is the same as `A` but the physical storage
  /// is the same as `()`. `sizeof (ghost A) = 0`.
  Ghost(Box<Type>),
  /// `(? T)` is the type of possibly-uninitialized `T`s. The typing predicate
  /// for this type is vacuous, but it has the same size as `T`, so overwriting with
  /// a `T` is possible.
  Uninit(Box<Type>),
  /// A propositional type, used for hypotheses.
  Prop(Box<Prop>),
  /// A user-defined type-former.
  User(AtomId, Box<[Type]>, Box<[Expr]>),
  /// The input token.
  Input,
  /// The output token.
  Output,
  /// A moved-away type.
  Moved(Box<Type>),
  /// A substitution into a type.
  Subst(Box<Type>, VarId, Box<Expr>),
  /// A type error that has been reported.
  Error,
}

impl Remap for TypeKind {
  type Target = Self;
  fn remap(&self, r: &mut Remapper) -> Self {
    match self {
      TypeKind::Unit => TypeKind::Unit,
      TypeKind::Bool => TypeKind::Bool,
      &TypeKind::Var(i) => TypeKind::Var(i),
      &TypeKind::Int(i) => TypeKind::Int(i),
      &TypeKind::UInt(i) => TypeKind::UInt(i),
      TypeKind::Array(ty, n) => TypeKind::Array(ty.remap(r), n.remap(r)),
      TypeKind::Own(ty) => TypeKind::Own(ty.remap(r)),
      TypeKind::Ref(lft, ty) => TypeKind::Ref(lft.clone(), ty.remap(r)),
      TypeKind::Shr(lft, ty) => TypeKind::Shr(lft.clone(), ty.remap(r)),
      TypeKind::RefSn(ty) => TypeKind::RefSn(ty.remap(r)),
      TypeKind::List(tys) => TypeKind::List(tys.remap(r)),
      TypeKind::Sn(e) => TypeKind::Sn(e.remap(r)),
      TypeKind::Struct(tys) => TypeKind::Struct(tys.remap(r)),
      TypeKind::And(tys) => TypeKind::And(tys.remap(r)),
      TypeKind::Or(tys) => TypeKind::Or(tys.remap(r)),
      TypeKind::If(c, t, e) => TypeKind::If(c.remap(r), t.remap(r), e.remap(r)),
      TypeKind::Match(c, brs) => TypeKind::Match(c.remap(r), brs.remap(r)),
      TypeKind::Ghost(ty) => TypeKind::Ghost(ty.remap(r)),
      TypeKind::Uninit(ty) => TypeKind::Uninit(ty.remap(r)),
      TypeKind::Prop(p) => TypeKind::Prop(p.remap(r)),
      TypeKind::User(f, tys, es) => TypeKind::User(f.remap(r), tys.remap(r), es.remap(r)),
      TypeKind::Input => TypeKind::Input,
      TypeKind::Output => TypeKind::Output,
      TypeKind::Moved(tys) => TypeKind::Moved(tys.remap(r)),
      TypeKind::Subst(ty, v, e) => TypeKind::Subst(ty.remap(r), *v, e.remap(r)),
      TypeKind::Error => TypeKind::Error,
    }
  }
}

/// A propositional expression.
pub type Prop = Spanned<PropKind>;

/// A separating proposition, which classifies hypotheses / proof terms.
#[derive(Debug, DeepSizeOf)]
pub enum PropKind {
  /// A true proposition.
  True,
  /// A false proposition.
  False,
  /// A universally quantified proposition.
  All(Box<[TuplePattern]>, Box<Prop>),
  /// An existentially quantified proposition.
  Ex(Box<[TuplePattern]>, Box<Prop>),
  /// Implication (plain, non-separating).
  Imp(Box<Prop>, Box<Prop>),
  /// Negation.
  Not(Box<Prop>),
  /// Conjunction (non-separating).
  And(Box<[Prop]>),
  /// Disjunction.
  Or(Box<[Prop]>),
  /// The empty heap.
  Emp,
  /// Separating conjunction.
  Sep(Box<[Prop]>),
  /// Separating implication.
  Wand(Box<Prop>, Box<Prop>),
  /// An (executable) boolean expression, interpreted as a pure proposition
  Pure(Box<Expr>),
  /// Equality (possibly non-decidable).
  Eq(Box<Expr>, Box<Expr>),
  /// A heap assertion `l |-> (v: T)`.
  Heap(Box<Expr>, Box<Expr>),
  /// An explicit typing assertion `[v : T]`.
  HasTy(Box<Expr>, Box<Type>),
  /// The move operator `|T|` on types.
  Moved(Box<Prop>),
  /// An embedded MM0 proposition of sort `wff`.
  Mm0(Mm0Expr<Expr>),
}

impl Remap for PropKind {
  type Target = Self;
  fn remap(&self, r: &mut Remapper) -> Self {
    match self {
      PropKind::True => PropKind::True,
      PropKind::False => PropKind::False,
      PropKind::All(p, q) => PropKind::All(p.remap(r), q.remap(r)),
      PropKind::Ex(p, q) => PropKind::Ex(p.remap(r), q.remap(r)),
      PropKind::Imp(p, q) => PropKind::Imp(p.remap(r), q.remap(r)),
      PropKind::Not(p) => PropKind::Not(p.remap(r)),
      PropKind::And(p) => PropKind::And(p.remap(r)),
      PropKind::Or(p) => PropKind::Or(p.remap(r)),
      PropKind::Emp => PropKind::Emp,
      PropKind::Sep(p) => PropKind::Sep(p.remap(r)),
      PropKind::Wand(p, q) => PropKind::Wand(p.remap(r), q.remap(r)),
      PropKind::Pure(p) => PropKind::Pure(p.remap(r)),
      PropKind::Eq(p, q) => PropKind::Eq(p.remap(r), q.remap(r)),
      PropKind::Heap(p, q) => PropKind::Heap(p.remap(r), q.remap(r)),
      PropKind::HasTy(p, q) => PropKind::HasTy(p.remap(r), q.remap(r)),
      PropKind::Moved(p) => PropKind::Moved(p.remap(r)),
      PropKind::Mm0(p) => PropKind::Mm0(p.remap(r)),
    }
  }
}

/// The type of variant, or well founded order that recursions decrease.
#[derive(Debug, DeepSizeOf)]
pub enum VariantType {
  /// This variant is a nonnegative natural number which decreases to 0.
  Down,
  /// This variant is a natural number or integer which increases while
  /// remaining less than this constant.
  UpLt(Expr),
  /// This variant is a natural number or integer which increases while
  /// remaining less than or equal to this constant.
  UpLe(Expr)
}

impl Remap for VariantType {
  type Target = Self;
  fn remap(&self, r: &mut Remapper) -> Self {
    match self {
      VariantType::Down => VariantType::Down,
      VariantType::UpLt(e) => VariantType::UpLt(e.remap(r)),
      VariantType::UpLe(e) => VariantType::UpLe(e.remap(r)),
    }
  }
}

/// A variant is a pure expression, together with a
/// well founded order that decreases on all calls.
pub type Variant = Spanned<(Expr, VariantType)>;

/// A label in a label group declaration. Individual labels in the group
/// are referred to by their index in the list.
#[derive(Debug, DeepSizeOf)]
pub struct Label {
  /// The arguments of the label
  pub args: Box<[Arg]>,
  /// The variant, for recursive calls
  pub variant: Option<Box<Variant>>,
  /// The code that is executed when you jump to the label
  pub body: Box<Expr>,
}

impl Remap for Label {
  type Target = Self;
  fn remap(&self, r: &mut Remapper) -> Self {
    Self {
      args: self.args.remap(r),
      variant: self.variant.remap(r),
      body: self.body.remap(r)
    }
  }
}

/// An expression or statement.
pub type Expr = Spanned<ExprKind>;

/// An expression or statement. A block is a list of expressions.
#[derive(Debug, DeepSizeOf)]
pub enum ExprKind {
  /// A `()` literal.
  Unit,
  /// A variable reference.
  Var(VarId),
  /// A user constant.
  Const(AtomId),
  /// A global variable.
  Global(AtomId),
  /// A number literal.
  Bool(bool),
  /// A number literal.
  Int(BigInt),
  /// A unary operation.
  Unop(Unop, Box<Expr>),
  /// A binary operation.
  Binop(Binop, Box<Expr>, Box<Expr>),
  /// `(sn x)` constructs the unique member of the type `(sn x)`.
  /// `(sn y h)` is also a member of `(sn x)` if `h` proves `y = x`.
  Sn(Box<Expr>, Option<Box<Expr>>),
  /// An index operation `(index a i h): T` where `a: (array T n)`,
  /// `i: nat`, and `h: i < n`.
  Index(Box<Expr>, Box<Expr>, Option<Box<Expr>>),
  /// If `x: (array T n)`, then `(slice x a b h): (array T b)` if
  /// `h` is a proof that `a + b <= n`.
  Slice(Box<(Expr, Expr, Expr)>, Option<Box<Expr>>),
  /// A projection operation `x.i: T` where
  /// `x: (T0, ..., T(n-1))` or `x: {f0: T0, ..., f(n-1): T(n-1)}`.
  Proj(Box<Expr>, FieldName),
  /// An deref operation `*x: T` where `x: &T`.
  Deref(Box<Expr>),
  /// `(list e1 ... en)` returns a tuple of the arguments.
  List(Vec<Expr>),
  /// A ghost expression.
  Ghost(Box<Expr>),
  /// Evaluates the expression as a pure expression, so it will not take
  /// ownership of the result.
  Place(Box<Expr>),
  /// `(& x)` constructs a reference to `x`.
  Ref(Box<Expr>),
  /// `(pure $e$)` embeds an MM0 expression `$e$` as the target type,
  /// one of the numeric types
  Mm0(Mm0Expr<Expr>),
  /// A type ascription.
  Typed(Box<Expr>, Box<Type>),
  /// A truncation / bit cast operation.
  As(Box<Expr>, Box<Type>),
  /// Combine an expression with a proof that it has the right type.
  Cast(Box<Expr>, Option<Box<Expr>>),
  /// Reinterpret an expression given a proof that it has the right type.
  Pun(Box<Expr>, Option<Box<Expr>>),
  /// An expression denoting an uninitialized value.
  Uninit,
  /// Return the size of a type.
  Sizeof(Box<Type>),
  /// Take the type of a variable.
  Typeof(Box<Expr>),
  /// `(assert p)` evaluates `p: bool` and returns a proof of `p`.
  Assert(Box<Expr>),
  /// A let binding.
  Let {
    /// A tuple pattern, containing variable bindings.
    lhs: TuplePattern,
    /// The expression to evaluate.
    rhs: Box<Expr>,
  },
  /// An assignment / mutation.
  Assign {
    /// A place (lvalue) to write to.
    lhs: Box<Expr>,
    /// The expression to evaluate.
    rhs: Box<Expr>,
  },
  /// A function call (or something that looks like one at parse time).
  Call {
    /// The function to call.
    f: Spanned<AtomId>,
    /// The type arguments.
    tys: Vec<Type>,
    /// The function arguments.
    args: Vec<Expr>,
    /// The variant, if needed.
    variant: Option<Box<Expr>>,
  },
  /// An entailment proof, which takes a proof of `P1 * ... * Pn => Q` and expressions proving
  /// `P1, ..., Pn` and is a hypothesis of type `Q`.
  Entail(LispVal, Box<[Expr]>),
  /// A block scope.
  Block(Vec<Expr>),
  /// A label, which looks exactly like a local function but has no independent stack frame.
  /// They are called like regular functions but can only appear in tail position.
  Label(VarId, Box<[Label]>),
  /// An if-then-else expression (at either block or statement level). The initial atom names
  /// a hypothesis that the expression is true in one branch and false in the other.
  If {
    /// The hypothesis name.
    hyp: Option<VarId>,
    /// The if condition.
    cond: Box<Expr>,
    /// The then case.
    then: Box<Expr>,
    /// The else case.
    els: Box<Expr>
  },
  /// A switch (pattern match) statement, given the initial expression and a list of match arms.
  Match(Box<Expr>, Box<[(Pattern, Expr)]>),
  /// A while loop.
  While {
    /// The name of this loop, which can be used as a target for jumps.
    label: VarId,
    /// A hypothesis that the condition is true in the loop and false after it.
    hyp: Option<VarId>,
    /// The loop condition.
    cond: Box<Expr>,
    /// The variant, which must decrease on every round around the loop.
    var: Option<Box<Variant>>,
    /// The body of the loop.
    body: Box<Expr>,
  },
  /// `(unreachable h)` takes a proof of false and undoes the current code path.
  Unreachable(Box<Expr>),
  /// `(lab e1 ... en)` jumps to label `lab` with `e1 ... en` as arguments.
  /// Here the `VarId` is the target label group, and the `u16` is the index
  /// in the label group.
  Jump(VarId, u16, Vec<Expr>, Option<Box<Expr>>),
  /// `(break lab e)` jumps out of the scope containing label `lab`,
  /// returning `e` as the result of the block. Unlike [`Jump`](Self::Jump),
  /// this does not contain a label index because breaking from any label
  /// in the group has the same effect.
  Break(VarId, Box<Expr>),
  /// `(return e1 ... en)` returns `e1, ..., en` from the current function.
  Return(Vec<Expr>),
  /// An inference hole `_`, which will give a compile error if it cannot be inferred
  /// but queries the compiler to provide a type context. The `bool` is true if this variable
  /// was created by the user through an explicit `_`, while compiler-generated inference
  /// variables have it set to false.
  Infer(bool),
  /// An upstream error.
  Error
}

impl Remap for ExprKind {
  type Target = Self;
  fn remap(&self, r: &mut Remapper) -> Self {
    match self {
      ExprKind::Unit => ExprKind::Unit,
      &ExprKind::Var(v) => ExprKind::Var(v),
      &ExprKind::Const(a) => ExprKind::Const(a.remap(r)),
      &ExprKind::Global(a) => ExprKind::Global(a.remap(r)),
      &ExprKind::Bool(b) => ExprKind::Bool(b),
      ExprKind::Int(n) => ExprKind::Int(n.clone()),
      ExprKind::Unop(op, e) => ExprKind::Unop(*op, e.remap(r)),
      ExprKind::Binop(op, e1, e2) => ExprKind::Binop(*op, e1.remap(r), e2.remap(r)),
      ExprKind::Sn(e, h) => ExprKind::Sn(e.remap(r), h.remap(r)),
      ExprKind::Index(a, i, h) => ExprKind::Index(a.remap(r), i.remap(r), h.remap(r)),
      ExprKind::Slice(e, h) => ExprKind::Slice(e.remap(r), h.remap(r)),
      ExprKind::Proj(e, i) => ExprKind::Proj(e.remap(r), *i),
      ExprKind::Deref(e) => ExprKind::Deref(e.remap(r)),
      ExprKind::List(e) => ExprKind::List(e.remap(r)),
      ExprKind::Ghost(e) => ExprKind::Ghost(e.remap(r)),
      ExprKind::Place(e) => ExprKind::Place(e.remap(r)),
      ExprKind::Ref(e) => ExprKind::Ref(e.remap(r)),
      ExprKind::Mm0(e) => ExprKind::Mm0(e.remap(r)),
      ExprKind::Typed(e, ty) => ExprKind::Typed(e.remap(r), ty.remap(r)),
      ExprKind::As(e, ty) => ExprKind::As(e.remap(r), ty.remap(r)),
      ExprKind::Cast(e, h) => ExprKind::Cast(e.remap(r), h.remap(r)),
      ExprKind::Pun(e, h) => ExprKind::Pun(e.remap(r), h.remap(r)),
      ExprKind::Uninit => ExprKind::Uninit,
      ExprKind::Sizeof(ty) => ExprKind::Sizeof(ty.remap(r)),
      ExprKind::Typeof(e) => ExprKind::Typeof(e.remap(r)),
      ExprKind::Assert(e) => ExprKind::Assert(e.remap(r)),
      ExprKind::Let { lhs, rhs } => ExprKind::Let { lhs: lhs.remap(r), rhs: rhs.remap(r) },
      ExprKind::Assign { lhs, rhs } => ExprKind::Assign { lhs: lhs.remap(r), rhs: rhs.remap(r) },
      ExprKind::Call { f, tys, args, variant } => ExprKind::Call {
        f: f.remap(r), tys: tys.remap(r), args: args.remap(r), variant: variant.remap(r) },
      ExprKind::Entail(p, q) => ExprKind::Entail(p.remap(r), q.remap(r)),
      ExprKind::Block(e) => ExprKind::Block(e.remap(r)),
      ExprKind::Label(v, e) => ExprKind::Label(*v, e.remap(r)),
      ExprKind::If { hyp, cond, then, els } => ExprKind::If {
        hyp: *hyp, cond: cond.remap(r), then: then.remap(r), els: els.remap(r) },
      ExprKind::Match(e, brs) => ExprKind::Match(e.remap(r), brs.remap(r)),
      ExprKind::While { label, hyp, cond, var, body } => ExprKind::While {
        label: *label, hyp: *hyp, cond: cond.remap(r), var: var.remap(r), body: body.remap(r) },
      ExprKind::Unreachable(e) => ExprKind::Unreachable(e.remap(r)),
      ExprKind::Jump(l, i, e, var) => ExprKind::Jump(*l, *i, e.remap(r), var.remap(r)),
      ExprKind::Break(v, e) => ExprKind::Break(*v, e.remap(r)),
      ExprKind::Return(e) => ExprKind::Return(e.remap(r)),
      &ExprKind::Infer(b) => ExprKind::Infer(b),
      ExprKind::Error => ExprKind::Error,
    }
  }
}

/// A field of a struct.
#[derive(Debug, DeepSizeOf)]
pub struct Field {
  /// The name of the field.
  pub name: AtomId,
  /// True if the field is computationally irrelevant.
  pub ghost: bool,
  /// The type of the field.
  pub ty: Type,
}

/// A procedure kind, which defines the different kinds of function-like declarations.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum ProcKind {
  /// A (pure) function, which generates a logic level function as well as code. (Body required.)
  Func,
  /// A procedure, which is opaque except for its type. (Body provided.)
  Proc,
  /// An intrinsic declaration, which is only here to put the function declaration in user code.
  /// The compiler will ensure this matches an existing intrinsic, and intrinsics cannot be
  /// called until they are declared using an `intrinsic` declaration.
  Intrinsic(Intrinsic),
}
crate::deep_size_0!(ProcKind);

/// A return value, after resolving `mut` / `out` annotations.
#[derive(Debug, DeepSizeOf)]
pub enum Ret {
  /// This is a regular argument, with the given argument pattern.
  Reg(TuplePattern),
  /// This is an anonymous `out`: `OutAnon(i, v)` means that argument `i`
  /// was marked as `mut` but there is no corresponding `out`,
  /// so this binder with name `v` was inserted to capture the outgoing value
  /// of the variable.
  OutAnon(u32, VarId),
  /// This is an `out` argument: `Out(i, pat)` means that this argument was marked as
  /// `out` corresponding to argument `i` in the inputs. `pat` contains the
  /// provided argument pattern.
  Out(u32, TuplePattern),
}

bitflags! {
  /// Attributes on function arguments.
  pub struct ArgAttr: u8 {
    /// A `(mut x)` argument, which is modified in the body and passed out
    /// via an `(out x x')` in the returns.
    const MUT = 1;
    /// An `(implicit x)` argument, which indicates that the variable will be
    /// inferred in applications.
    const IMPLICIT = 2;
    /// A `(global x)` argument, which indicates that the variable is not passed directly
    /// but is instead sourced from a global variable of the same name.
    const GLOBAL = 4;
  }
}
crate::deep_size_0!(ArgAttr);

impl Remap for ArgAttr {
  type Target = Self;
  fn remap(&self, _: &mut Remapper) -> Self { *self }
}

/// A procedure (or function or intrinsic), a top level item similar to function declarations in C.
#[derive(Debug, DeepSizeOf)]
pub struct Proc {
  /// The type of declaration: `func`, `proc`, or `intrinsic`.
  pub kind: ProcKind,
  /// The name of the procedure.
  pub name: Spanned<AtomId>,
  /// The number of type arguments
  pub tyargs: u32,
  /// The arguments of the procedure.
  pub args: Box<[Arg]>,
  /// The return values of the procedure. (Functions and procedures return multiple values in MMC.)
  pub rets: Vec<Ret>,
  /// The variant, used for recursive functions.
  pub variant: Option<Box<Variant>>,
  /// The body of the procedure.
  pub body: Vec<Expr>
}

/// A top level program item. (A program AST is a list of program items.)
pub type Item = Spanned<ItemKind>;

/// A top level program item. (A program AST is a list of program items.)
#[derive(Debug, DeepSizeOf)]
pub enum ItemKind {
  /// A procedure, behind an Arc so it can be cheaply copied.
  Proc(Proc),
  /// A global variable declaration.
  Global {
    /// The variable(s) being declared
    lhs: TuplePattern,
    /// The value of the declaration
    rhs: Expr,
  },
  /// A constant declaration.
  Const {
    /// The constant(s) being declared
    lhs: TuplePattern,
    /// The value of the declaration
    rhs: Expr,
  },
  /// A type definition.
  Typedef {
    /// The name of the newly declared type
    name: Spanned<AtomId>,
    /// The number of type arguments
    tyargs: u32,
    /// The arguments of the type declaration, for a parametric type
    args: Box<[Arg]>,
    /// The value of the declaration (another type)
    val: Type,
  },
}