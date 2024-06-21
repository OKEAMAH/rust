use chalk_ir::{cast::Cast as _, BoundVar, DebruijnIndex};
use hir_def::{
    db::DefDatabase,
    generics::{
        GenericParamDataRef, GenericParams, LifetimeParamData, TypeOrConstParamData,
        TypeParamProvenance,
    },
    ConstParamId, GenericDefId, GenericParamId, ItemContainerId, LifetimeParamId, Lookup,
    TypeOrConstParamId, TypeParamId,
};
use intern::Interned;

use crate::{db::HirDatabase, Interner, Substitution};

pub(crate) fn generics(db: &dyn DefDatabase, def: GenericDefId) -> Generics {
    let parent_generics = parent_generic_def(db, def).map(|def| Box::new(generics(db, def)));
    Generics { def, params: db.generic_params(def), parent_generics }
}
#[derive(Clone, Debug)]
pub(crate) struct Generics {
    def: GenericDefId,
    pub(crate) params: Interned<GenericParams>,
    parent_generics: Option<Box<Generics>>,
}

impl Generics {
    pub(crate) fn iter_id(&self) -> impl Iterator<Item = GenericParamId> + '_ {
        self.iter().map(|(id, _)| id)
    }

    pub(crate) fn def(&self) -> GenericDefId {
        self.def
    }

    /// Iterator over types and const params of self, then parent.
    pub(crate) fn iter<'a>(
        &'a self,
    ) -> impl DoubleEndedIterator<Item = (GenericParamId, GenericParamDataRef<'a>)> + 'a {
        let from_toc_id = |it: &'a Generics| {
            move |(local_id, p): (_, &'a TypeOrConstParamData)| {
                let id = TypeOrConstParamId { parent: it.def, local_id };
                match p {
                    TypeOrConstParamData::TypeParamData(p) => (
                        GenericParamId::TypeParamId(TypeParamId::from_unchecked(id)),
                        GenericParamDataRef::TypeParamData(p),
                    ),
                    TypeOrConstParamData::ConstParamData(p) => (
                        GenericParamId::ConstParamId(ConstParamId::from_unchecked(id)),
                        GenericParamDataRef::ConstParamData(p),
                    ),
                }
            }
        };

        let from_lt_id = |it: &'a Generics| {
            move |(local_id, p): (_, &'a LifetimeParamData)| {
                (
                    GenericParamId::LifetimeParamId(LifetimeParamId { parent: it.def, local_id }),
                    GenericParamDataRef::LifetimeParamData(p),
                )
            }
        };

        let lt_iter = self.params.iter_lt().map(from_lt_id(self));
        self.params
            .iter_type_or_consts()
            .map(from_toc_id(self))
            .chain(lt_iter)
            .chain(self.iter_parent())
    }

    /// Iterate over types and const params without parent params.
    pub(crate) fn iter_self<'a>(
        &'a self,
    ) -> impl DoubleEndedIterator<Item = (GenericParamId, GenericParamDataRef<'a>)> + 'a {
        let from_toc_id = |it: &'a Generics| {
            move |(local_id, p): (_, &'a TypeOrConstParamData)| {
                let id = TypeOrConstParamId { parent: it.def, local_id };
                match p {
                    TypeOrConstParamData::TypeParamData(p) => (
                        GenericParamId::TypeParamId(TypeParamId::from_unchecked(id)),
                        GenericParamDataRef::TypeParamData(p),
                    ),
                    TypeOrConstParamData::ConstParamData(p) => (
                        GenericParamId::ConstParamId(ConstParamId::from_unchecked(id)),
                        GenericParamDataRef::ConstParamData(p),
                    ),
                }
            }
        };

        let from_lt_id = |it: &'a Generics| {
            move |(local_id, p): (_, &'a LifetimeParamData)| {
                (
                    GenericParamId::LifetimeParamId(LifetimeParamId { parent: it.def, local_id }),
                    GenericParamDataRef::LifetimeParamData(p),
                )
            }
        };

        self.params
            .iter_type_or_consts()
            .map(from_toc_id(self))
            .chain(self.params.iter_lt().map(from_lt_id(self)))
    }

    /// Iterator over types and const params of parent.
    pub(crate) fn iter_parent(
        &self,
    ) -> impl DoubleEndedIterator<Item = (GenericParamId, GenericParamDataRef<'_>)> + '_ {
        self.parent_generics().into_iter().flat_map(|it| {
            let from_toc_id = move |(local_id, p)| {
                let p: &_ = p;
                let id = TypeOrConstParamId { parent: it.def, local_id };
                match p {
                    TypeOrConstParamData::TypeParamData(p) => (
                        GenericParamId::TypeParamId(TypeParamId::from_unchecked(id)),
                        GenericParamDataRef::TypeParamData(p),
                    ),
                    TypeOrConstParamData::ConstParamData(p) => (
                        GenericParamId::ConstParamId(ConstParamId::from_unchecked(id)),
                        GenericParamDataRef::ConstParamData(p),
                    ),
                }
            };

            let from_lt_id = move |(local_id, p): (_, _)| {
                (
                    GenericParamId::LifetimeParamId(LifetimeParamId { parent: it.def, local_id }),
                    GenericParamDataRef::LifetimeParamData(p),
                )
            };
            let lt_iter = it.params.iter_lt().map(from_lt_id);
            it.params.iter_type_or_consts().map(from_toc_id).chain(lt_iter)
        })
    }

    /// Returns total number of generic parameters in scope, including those from parent.
    pub(crate) fn len(&self) -> usize {
        let parent = self.parent_generics().map_or(0, Generics::len);
        let child = self.params.len();
        parent + child
    }

    /// Returns numbers of generic parameters excluding those from parent.
    pub(crate) fn len_self(&self) -> usize {
        self.params.len()
    }

    /// Returns number of generic parameter excluding those from parent
    fn len_type_and_const_params(&self) -> usize {
        self.params.type_or_consts.len()
    }

    /// (parent total, self param, type params, const params, impl trait list, lifetimes)
    pub(crate) fn provenance_split(&self) -> (usize, usize, usize, usize, usize, usize) {
        let mut self_params = 0;
        let mut type_params = 0;
        let mut impl_trait_params = 0;
        let mut const_params = 0;
        let mut lifetime_params = 0;
        self.params.iter_type_or_consts().for_each(|(_, data)| match data {
            TypeOrConstParamData::TypeParamData(p) => match p.provenance {
                TypeParamProvenance::TypeParamList => type_params += 1,
                TypeParamProvenance::TraitSelf => self_params += 1,
                TypeParamProvenance::ArgumentImplTrait => impl_trait_params += 1,
            },
            TypeOrConstParamData::ConstParamData(_) => const_params += 1,
        });

        self.params.iter_lt().for_each(|(_, _)| lifetime_params += 1);

        let parent_len = self.parent_generics().map_or(0, Generics::len);
        (parent_len, self_params, type_params, const_params, impl_trait_params, lifetime_params)
    }

    pub(crate) fn type_or_const_param_idx(&self, param: TypeOrConstParamId) -> Option<usize> {
        Some(self.find_type_or_const_param(param)?.0)
    }

    fn find_type_or_const_param(
        &self,
        param: TypeOrConstParamId,
    ) -> Option<(usize, &TypeOrConstParamData)> {
        if param.parent == self.def {
            let idx = param.local_id.into_raw().into_u32() as usize;
            if idx >= self.params.type_or_consts.len() {
                return None;
            }
            Some((idx, &self.params.type_or_consts[param.local_id]))
        } else {
            self.parent_generics()
                .and_then(|g| g.find_type_or_const_param(param))
                // Remember that parent parameters come after parameters for self.
                .map(|(idx, data)| (self.len_self() + idx, data))
        }
    }

    pub(crate) fn lifetime_idx(&self, lifetime: LifetimeParamId) -> Option<usize> {
        Some(self.find_lifetime(lifetime)?.0)
    }

    fn find_lifetime(&self, lifetime: LifetimeParamId) -> Option<(usize, &LifetimeParamData)> {
        if lifetime.parent == self.def {
            let idx = lifetime.local_id.into_raw().into_u32() as usize;
            if idx >= self.params.lifetimes.len() {
                return None;
            }
            Some((
                self.len_type_and_const_params() + idx,
                &self.params.lifetimes[lifetime.local_id],
            ))
        } else {
            self.parent_generics()
                .and_then(|g| g.find_lifetime(lifetime))
                .map(|(idx, data)| (self.len_self() + idx, data))
        }
    }

    pub(crate) fn parent_generics(&self) -> Option<&Generics> {
        self.parent_generics.as_deref()
    }

    pub(crate) fn parent_or_self(&self) -> &Generics {
        self.parent_generics.as_deref().unwrap_or(self)
    }

    /// Returns a Substitution that replaces each parameter by a bound variable.
    pub(crate) fn bound_vars_subst(
        &self,
        db: &dyn HirDatabase,
        debruijn: DebruijnIndex,
    ) -> Substitution {
        Substitution::from_iter(
            Interner,
            self.iter_id().enumerate().map(|(idx, id)| match id {
                GenericParamId::ConstParamId(id) => BoundVar::new(debruijn, idx)
                    .to_const(Interner, db.const_param_ty(id))
                    .cast(Interner),
                GenericParamId::TypeParamId(_) => {
                    BoundVar::new(debruijn, idx).to_ty(Interner).cast(Interner)
                }
                GenericParamId::LifetimeParamId(_) => {
                    BoundVar::new(debruijn, idx).to_lifetime(Interner).cast(Interner)
                }
            }),
        )
    }

    /// Returns a Substitution that replaces each parameter by itself (i.e. `Ty::Param`).
    pub(crate) fn placeholder_subst(&self, db: &dyn HirDatabase) -> Substitution {
        Substitution::from_iter(
            Interner,
            self.iter_id().map(|id| match id {
                GenericParamId::TypeParamId(id) => {
                    crate::to_placeholder_idx(db, id.into()).to_ty(Interner).cast(Interner)
                }
                GenericParamId::ConstParamId(id) => crate::to_placeholder_idx(db, id.into())
                    .to_const(Interner, db.const_param_ty(id))
                    .cast(Interner),
                GenericParamId::LifetimeParamId(id) => {
                    crate::lt_to_placeholder_idx(db, id).to_lifetime(Interner).cast(Interner)
                }
            }),
        )
    }
}

fn parent_generic_def(db: &dyn DefDatabase, def: GenericDefId) -> Option<GenericDefId> {
    let container = match def {
        GenericDefId::FunctionId(it) => it.lookup(db).container,
        GenericDefId::TypeAliasId(it) => it.lookup(db).container,
        GenericDefId::ConstId(it) => it.lookup(db).container,
        GenericDefId::EnumVariantId(it) => return Some(it.lookup(db).parent.into()),
        GenericDefId::AdtId(_)
        | GenericDefId::TraitId(_)
        | GenericDefId::ImplId(_)
        | GenericDefId::TraitAliasId(_) => return None,
    };

    match container {
        ItemContainerId::ImplId(it) => Some(it.into()),
        ItemContainerId::TraitId(it) => Some(it.into()),
        ItemContainerId::ModuleId(_) | ItemContainerId::ExternBlockId(_) => None,
    }
}
