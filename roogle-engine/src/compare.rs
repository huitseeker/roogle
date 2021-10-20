use std::{
    cmp::{max, min},
    collections::HashMap,
};

use levenshtein::levenshtein;
use rustdoc_types as types;
use smallvec::{smallvec, SmallVec};
use tracing::{instrument, trace};

use crate::query::*;

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub enum Similarity {
    /// Represents how digitally similar two objects are.
    Discrete(DiscreteSimilarity),

    /// Represents how analogly similar two objects are.
    Continuous(f32),
}

use Similarity::*;

#[derive(Debug, Clone, PartialEq)]
pub struct Similarities(pub SmallVec<[Similarity; 10]>);

impl Similarities {
    /// Calculate objective similarity for sorting.
    pub fn score(&self) -> f32 {
        let sum: f32 = self
            .0
            .iter()
            .map(|sim| match sim {
                Discrete(Equivalent) => 0.0,
                Discrete(Subequal) => 0.25,
                Discrete(Different) => 1.0,
                Continuous(s) => *s,
            })
            .sum();
        sum / self.0.len() as f32
    }
}

impl PartialOrd for Similarities {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        (self.score()).partial_cmp(&other.score())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DiscreteSimilarity {
    /// Indicates that two types are the same.
    ///
    /// For example:
    /// - `i32` and `i32`
    /// - `Result<i32, ()>` and `Result<i32, ()>`
    Equivalent,

    /// Indicates that two types are partially equal.
    ///
    /// For example:
    /// - an unbound generic type `T` and `i32`
    /// - an unbound generic type `T` and `Option<U>`
    Subequal,

    /// Indicates that two types are not similar at all.
    ///
    /// For example:
    /// - `i32` and `Option<bool>`
    Different,
}

use DiscreteSimilarity::*;

pub trait Compare<Rhs> {
    fn compare(
        &self,
        rhs: &Rhs,
        generics: &mut types::Generics,
        substs: &mut HashMap<String, Type>,
    ) -> SmallVec<[Similarity; 10]>;
}

impl Compare<types::Item> for Query {
    #[instrument]
    fn compare(
        &self,
        item: &types::Item,
        generics: &mut types::Generics,
        substs: &mut HashMap<String, Type>,
    ) -> SmallVec<[Similarity; 10]> {
        let mut sims = smallvec![];

        match (&self.name, &item.name) {
            (Some(q), Some(i)) => sims.append(&mut q.compare(i, generics, substs)),
            (Some(_), None) => sims.push(Discrete(Different)),
            _ => {}
        }
        trace!(?sims);

        if let Some(ref kind) = self.kind {
            sims.append(&mut kind.compare(&item.inner, generics, substs))
        }
        trace!(?sims);

        sims
    }
}

impl Compare<String> for Symbol {
    #[instrument]
    fn compare(
        &self,
        symbol: &String,
        _: &mut types::Generics,
        _: &mut HashMap<String, Type>,
    ) -> SmallVec<[Similarity; 10]> {
        use std::cmp::max;
        smallvec![Continuous(
            levenshtein(self, symbol) as f32 / max(self.len(), symbol.len()) as f32
        )]
    }
}

impl Compare<types::ItemEnum> for QueryKind {
    #[instrument]
    fn compare(
        &self,
        kind: &types::ItemEnum,
        generics: &mut types::Generics,
        substs: &mut HashMap<String, Type>,
    ) -> SmallVec<[Similarity; 10]> {
        use types::ItemEnum::*;
        use QueryKind::*;

        match (self, kind) {
            (FunctionQuery(q), Function(i)) => q.compare(i, generics, substs),
            (FunctionQuery(q), Method(i)) => q.compare(i, generics, substs),
            (FunctionQuery(_), _) => smallvec![Discrete(Different)],
        }
    }
}

impl Compare<types::Function> for Function {
    #[instrument]
    fn compare(
        &self,
        function: &types::Function,
        generics: &mut types::Generics,
        substs: &mut HashMap<String, Type>,
    ) -> SmallVec<[Similarity; 10]> {
        generics
            .params
            .append(&mut function.generics.params.clone());
        generics
            .where_predicates
            .append(&mut function.generics.where_predicates.clone());
        self.decl.compare(&function.decl, generics, substs)
    }
}

impl Compare<types::Method> for Function {
    #[instrument]
    fn compare(
        &self,
        method: &types::Method,
        generics: &mut types::Generics,
        substs: &mut HashMap<String, Type>,
    ) -> SmallVec<[Similarity; 10]> {
        generics.params.append(&mut method.generics.params.clone());
        generics
            .where_predicates
            .append(&mut method.generics.where_predicates.clone());
        self.decl.compare(&method.decl, generics, substs)
    }
}

impl Compare<types::FnDecl> for FnDecl {
    #[instrument]
    fn compare(
        &self,
        decl: &types::FnDecl,
        generics: &mut types::Generics,
        substs: &mut HashMap<String, Type>,
    ) -> SmallVec<[Similarity; 10]> {
        let mut sims = smallvec![];

        if let Some(ref inputs) = self.inputs {
            inputs.iter().enumerate().for_each(|(idx, q)| {
                if let Some(i) = decl.inputs.get(idx) {
                    sims.append(&mut q.compare(i, generics, substs))
                }
            });

            if inputs.len() != decl.inputs.len() {
                // FIXME: Replace this line below with `usize::abs_diff` once it got stablized.
                let abs_diff =
                    max(inputs.len(), decl.inputs.len()) - min(inputs.len(), decl.inputs.len());
                sims.append::<[Similarity; 10]>(&mut smallvec![Discrete(Different); abs_diff])
            } else if inputs.is_empty() && decl.inputs.is_empty() {
                sims.push(Discrete(Equivalent));
            }
        }
        trace!(?sims);

        if let Some(ref output) = self.output {
            sims.append(&mut output.compare(&decl.output, generics, substs));
        }
        trace!(?sims);

        sims
    }
}

impl Compare<(String, types::Type)> for Argument {
    #[instrument]
    fn compare(
        &self,
        arg: &(String, types::Type),
        generics: &mut types::Generics,
        substs: &mut HashMap<String, Type>,
    ) -> SmallVec<[Similarity; 10]> {
        let mut sims = smallvec![];

        if let Some(ref name) = self.name {
            sims.append(&mut name.compare(&arg.0, generics, substs));
        }
        trace!(?sims);

        if let Some(ref type_) = self.ty {
            sims.append(&mut type_.compare(&arg.1, generics, substs));
        }
        trace!(?sims);

        sims
    }
}

impl Compare<Option<types::Type>> for FnRetTy {
    #[instrument]
    fn compare(
        &self,
        ret_ty: &Option<types::Type>,
        generics: &mut types::Generics,
        substs: &mut HashMap<String, Type>,
    ) -> SmallVec<[Similarity; 10]> {
        match (self, ret_ty) {
            (FnRetTy::Return(q), Some(i)) => q.compare(i, generics, substs),
            (FnRetTy::DefaultReturn, None) => smallvec![Discrete(Equivalent)],
            _ => smallvec![Discrete(Different)],
        }
    }
}

impl Compare<types::Type> for Type {
    #[instrument]
    fn compare(
        &self,
        type_: &types::Type,
        generics: &mut types::Generics,
        substs: &mut HashMap<String, Type>,
    ) -> SmallVec<[Similarity; 10]> {
        use {crate::query::Type::*, types::Type};

        match (self, type_) {
            (q, Type::Generic(i)) if i == "Self" => {
                let mut i = None;
                for where_predicate in &generics.where_predicates {
                    if let types::WherePredicate::EqPredicate {
                        lhs: types::Type::Generic(lhs),
                        rhs,
                    } = where_predicate
                    {
                        if lhs == "Self" {
                            i = Some(rhs).cloned();
                            break;
                        }
                    }
                }
                let i = &i.unwrap(); // SAFETY: `Self` only appears in definitions of associated items.
                q.compare(i, generics, substs)
            }
            (q, types::Type::Generic(i)) => match substs.get(i) {
                Some(i) => {
                    if q == i {
                        smallvec![Discrete(Equivalent)]
                    } else {
                        smallvec![Discrete(Different)]
                    }
                }
                None => {
                    substs.insert(i.clone(), q.clone());
                    smallvec![Discrete(Subequal)]
                }
            },
            (Tuple(q), Type::Tuple(i)) => {
                let mut sims = q
                    .iter()
                    .zip(i.iter())
                    .filter_map(|(q, i)| q.as_ref().map(|q| q.compare(i, generics, substs)))
                    .flatten()
                    .collect::<SmallVec<_>>();

                // They are both tuples.
                sims.push(Discrete(Equivalent));

                // FIXME: Replace this line below with `usize::abs_diff` once it got stablized.
                let abs_diff = max(q.len(), i.len()) - min(q.len(), i.len());
                sims.append::<[Similarity; 10]>(&mut smallvec![Discrete(Different); abs_diff]);

                sims
            }
            (Slice(q), Type::Slice(i)) => {
                // They are both slices.
                let mut sims = smallvec![Discrete(Equivalent)];

                if let Some(q) = q {
                    sims.append(&mut q.compare(i, generics, substs));
                }

                sims
            }
            (
                RawPointer {
                    mutable: q_mut,
                    type_: q,
                },
                Type::RawPointer {
                    mutable: i_mut,
                    type_: i,
                },
            )
            | (
                BorrowedRef {
                    mutable: q_mut,
                    type_: q,
                },
                Type::BorrowedRef {
                    mutable: i_mut,
                    type_: i,
                    ..
                },
            ) => {
                if q_mut == i_mut {
                    q.compare(i, generics, substs)
                } else {
                    let mut sims = q.compare(i, generics, substs);
                    sims.push(Discrete(Subequal));
                    sims
                }
            }
            (q, Type::RawPointer { type_: i, .. } | Type::BorrowedRef { type_: i, .. }) => {
                let mut sims = q.compare(i, generics, substs);
                sims.push(Discrete(Subequal));
                sims
            }
            (RawPointer { type_: q, .. } | BorrowedRef { type_: q, .. }, i) => {
                let mut sims = q.compare(i, generics, substs);
                sims.push(Discrete(Subequal));
                sims
            }
            (
                UnresolvedPath {
                    name: q,
                    args: q_args,
                },
                Type::ResolvedPath {
                    name: i,
                    args: i_args,
                    ..
                },
            ) => {
                let mut sims = q.compare(i, generics, substs);

                match (q_args, i_args) {
                    (Some(q), Some(i)) => match (&**q, &**i) {
                        (
                            GenericArgs::AngleBracketed { args: ref q },
                            types::GenericArgs::AngleBracketed { args: ref i, .. },
                        ) => {
                            let q = q.iter().map(|q| {
                                q.as_ref().map(|q| match q {
                                    GenericArg::Type(q) => q,
                                })
                            });
                            let i = i.iter().map(|i| match i {
                                types::GenericArg::Type(t) => Some(t),
                                _ => None,
                            });
                            q.zip(i).for_each(|(q, i)| match (q, i) {
                                (Some(q), Some(i)) => {
                                    sims.append(&mut q.compare(i, generics, substs))
                                }
                                (Some(_), None) => sims.push(Discrete(Different)),
                                (None, _) => {}
                            });
                        }
                        // TODO: Support `GenericArgs::Parenthesized`.
                        (_, _) => {}
                    },
                    (Some(q), None) => {
                        let GenericArgs::AngleBracketed { args: ref q } = **q;
                        sims.append::<[Similarity; 10]>(
                            &mut smallvec![Discrete(Different); q.len()],
                        )
                    }
                    (None, _) => {}
                }

                sims
            }
            (Primitive(q), Type::Primitive(i)) => q.compare(i, generics, substs),
            _ => smallvec![Discrete(Different)],
        }
    }
}

impl Compare<String> for PrimitiveType {
    #[instrument]
    fn compare(
        &self,
        prim_ty: &String,
        _: &mut types::Generics,
        _: &mut HashMap<String, Type>,
    ) -> SmallVec<[Similarity; 10]> {
        if self.as_str() == prim_ty {
            smallvec![Discrete(Equivalent)]
        } else {
            smallvec![Discrete(Different)]
        }
    }
}