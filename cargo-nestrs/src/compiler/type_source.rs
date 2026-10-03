//! Source type paths preserve compiler crate identities and Cargo extern names.
//!
//! Rustc's diagnostic printer uses canonical crate names. Those names are not
//! necessarily in the application's extern prelude, and two versions may have
//! the same name. Reuse rustc's typed printer and visible re-export traversal,
//! but select each crate prefix by its actual loaded artifact identity.

use rustc_hir::def::{DefKind, Namespace};
use rustc_hir::def_id::{CrateNum, DefId, LOCAL_CRATE, LocalDefId};
use rustc_hir::definitions::{DefPathData, DefPathDataName, DisambiguatedDefPathData};
use rustc_middle::ty::print::{
    FmtPrinter, PrettyPrinter, Print, PrintError, Printer, WrapBinderMode,
};
use rustc_middle::ty::{self, GenericArg, Ty, TyCtxt, TypeFoldable, TypeFolder, Upcast as _};
use rustc_span::Symbol;
use std::collections::HashMap;
use std::fmt::{self, Write};
use std::rc::Rc;

/// One analysis owns this mapping; artifact paths are canonicalized once, not
/// for every comparison or projection emitted from its candidate set.
pub struct SourceTypes<'tcx> {
    tcx: TyCtxt<'tcx>,
    crate_names: Rc<HashMap<CrateNum, String>>,
}

impl<'tcx> SourceTypes<'tcx> {
    pub fn new(tcx: TyCtxt<'tcx>) -> Self {
        Self {
            tcx,
            crate_names: Rc::new(extern_names(tcx)),
        }
    }

    pub fn render(&self, ty: Ty<'tcx>) -> String {
        self.render_source(ty).0
    }

    /// 只有实际打印路径的每个 crate 根都能被当前源码引用，才交付生成用类型。
    ///
    /// rustc 的诊断打印器可以为传递依赖输出规范 crate 名，但该名字未必存在于
    /// 当前 extern prelude。不能把“metadata 中可见”误认为“源码中可以命名”。
    /// 这里检查的是 visible-parent 重导出选择之后的实际路径，因此不会排除
    /// 通过直接依赖重导出的传递类型，也会检查 trait 泛型参数里的隐藏类型。
    pub fn render_if_nameable(&self, ty: Ty<'tcx>) -> Option<String> {
        let (source, nameable) = self.render_source(ty);
        nameable.then_some(source)
    }

    fn render_source(&self, ty: Ty<'tcx>) -> (String, bool) {
        // Diagnostic printing restarts its region-name allocator for each
        // binder. Give every bound region a unique source name first, including
        // regions captured from an outer binder. Only this printable type copy
        // changes; candidate identity and type checks keep the original Ty.
        let ty = ty.fold_with(&mut SourceRegions {
            tcx: self.tcx,
            binders: Vec::new(),
            next_binder: 0,
        });
        let mut printer = SourcePrinter {
            tcx: self.tcx,
            output: String::new(),
            empty_path: false,
            in_value: false,
            crate_names: self.crate_names.clone(),
            nameable: true,
        };
        rustc_middle::ty::print::with_no_trimmed_paths!(printer.print_type(ty))
            .expect("writing a type to String cannot fail");
        (printer.output, printer.nameable)
    }
}

struct SourceRegions<'tcx> {
    tcx: TyCtxt<'tcx>,
    binders: Vec<usize>,
    next_binder: usize,
}

fn region_name(binder: usize, variable: usize) -> Symbol {
    Symbol::intern(&format!("'__nestrs_{binder}_{variable}"))
}

impl<'tcx> TypeFolder<TyCtxt<'tcx>> for SourceRegions<'tcx> {
    fn cx(&self) -> TyCtxt<'tcx> {
        self.tcx
    }

    fn fold_binder<T: TypeFoldable<TyCtxt<'tcx>>>(
        &mut self,
        binder: ty::Binder<'tcx, T>,
    ) -> ty::Binder<'tcx, T> {
        let id = self.next_binder;
        self.next_binder += 1;
        self.binders.push(id);
        let variables =
            self.tcx
                .mk_bound_variable_kinds_from_iter(binder.bound_vars().iter().enumerate().map(
                    |(index, kind)| {
                        if matches!(kind, ty::BoundVariableKind::Region(_)) {
                            ty::BoundVariableKind::Region(ty::BoundRegionKind::NamedForPrinting(
                                region_name(id, index),
                            ))
                        } else {
                            kind
                        }
                    },
                ));
        let value = binder.skip_binder().fold_with(self);
        self.binders.pop();
        ty::Binder::bind_with_vars(value, variables)
    }

    fn fold_region(&mut self, region: ty::Region<'tcx>) -> ty::Region<'tcx> {
        let ty::ReBound(ty::BoundVarIndexKind::Bound(depth), bound) = region.kind() else {
            return region;
        };
        let Some(index) = self.binders.len().checked_sub(depth.as_usize() + 1) else {
            return region;
        };
        ty::Region::new_bound(
            self.tcx,
            depth,
            ty::BoundRegion {
                var: bound.var,
                kind: ty::BoundRegionKind::NamedForPrinting(region_name(
                    self.binders[index],
                    bound.var.as_usize(),
                )),
            },
        )
    }
}

fn extern_names(tcx: TyCtxt<'_>) -> HashMap<CrateNum, String> {
    let mut names = HashMap::new();
    for &krate in tcx.crates(()) {
        let loaded = tcx.used_crate_source(krate);
        let artifacts: Vec<_> = loaded
            .paths()
            .map(|path| path.canonicalize().unwrap_or_else(|_| path.clone()))
            .collect();
        for (name, entry) in tcx.sess.opts.externs.iter() {
            if entry.add_prelude
                && entry.files().is_some_and(|mut files| {
                    files.any(|path| artifacts.contains(path.canonicalized()))
                })
            {
                names.insert(krate, name.clone());
                break;
            }
        }
    }
    names
}

struct SourcePrinter<'tcx> {
    tcx: TyCtxt<'tcx>,
    output: String,
    empty_path: bool,
    in_value: bool,
    crate_names: Rc<HashMap<CrateNum, String>>,
    nameable: bool,
}

impl fmt::Write for SourcePrinter<'_> {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        self.output.push_str(value);
        Ok(())
    }
}

impl<'tcx> Printer<'tcx> for SourcePrinter<'tcx> {
    fn tcx<'a>(&'a self) -> TyCtxt<'tcx> {
        self.tcx
    }

    fn print_def_path(
        &mut self,
        definition: DefId,
        args: &'tcx [GenericArg<'tcx>],
    ) -> Result<(), PrintError> {
        if args.is_empty() && self.try_print_visible_def_path(definition)? {
            return Ok(());
        }
        self.default_print_def_path(definition, args)
    }

    fn print_region(&mut self, region: ty::Region<'tcx>) -> Result<(), PrintError> {
        self.write_str(&FmtPrinter::print_string(
            self.tcx,
            Namespace::TypeNS,
            |printer| printer.print_region(region),
        )?)
    }

    fn print_type(&mut self, ty: Ty<'tcx>) -> Result<(), PrintError> {
        self.pretty_print_type(ty)
    }

    fn print_dyn_existential(
        &mut self,
        predicates: &'tcx ty::List<ty::PolyExistentialPredicate<'tcx>>,
    ) -> Result<(), PrintError> {
        // rustc's diagnostic projection printer writes the associated item's
        // Symbol directly, bypassing print_path_with_simple. Only these names
        // need a source-specific path; retain the standard Fn(...) sugar and
        // other diagnostic formatting whenever every binding name is legal.
        if !predicates.projection_bounds().any(|projection| {
            self.identifier_needs_raw(self.tcx.item_name(projection.skip_binder().def_id))
        }) {
            return self.pretty_print_dyn_existential(predicates);
        }

        let mut first = true;
        if let Some(bound_principal) = predicates.principal() {
            self.wrap_binder(
                &bound_principal,
                WrapBinderMode::ForAll,
                |principal, printer| {
                    let tcx = printer.tcx;
                    printer.print_def_path(principal.def_id, &[])?;
                    let principal_with_self =
                        principal.with_self_ty(tcx, tcx.types.trait_object_dummy_self);
                    let args = tcx
                        .generics_of(principal.def_id)
                        .own_args_no_defaults(tcx, principal_with_self.args);

                    // A supertrait may already bind an associated type under its
                    // own lifetime binder. Reprinting that equality on the outer
                    // object can be invalid, so preserve rustc's typed implication
                    // check instead of reconstructing bindings from display text.
                    let clause: ty::Clause<'tcx> = bound_principal
                        .with_self_ty(tcx, tcx.types.trait_object_dummy_self)
                        .upcast(tcx);
                    let implied: Vec<_> = ty::elaborate::elaborate(tcx, [clause])
                        .filter_only_self()
                        .filter_map(|clause| clause.as_projection_clause())
                        .map(|projection| {
                            tcx.erase_and_anonymize_regions(projection.map_bound(|projection| {
                                ty::ExistentialProjection::erase_self_ty(tcx, projection)
                            }))
                        })
                        .collect();
                    let mut projections: Vec<_> = predicates
                        .projection_bounds()
                        .filter(|projection| {
                            !implied.contains(&tcx.erase_and_anonymize_regions(*projection))
                        })
                        .map(|projection| projection.skip_binder())
                        .collect();
                    projections.sort_by_cached_key(|projection| {
                        tcx.item_name(projection.def_id).to_string()
                    });

                    if !args.is_empty() || !projections.is_empty() {
                        printer.generic_delimiters(|printer| {
                            printer.comma_sep(args.iter().copied())?;
                            let mut separator = !args.is_empty();
                            for projection in projections {
                                if separator {
                                    printer.write_str(", ")?;
                                }
                                separator = true;
                                printer.print_source_projection(projection)?;
                            }
                            Ok(())
                        })?;
                    }
                    Ok(())
                },
            )?;
            first = false;
        }
        let mut auto_traits: Vec<_> = predicates.auto_traits().collect();
        auto_traits.sort_by_cached_key(|definition| self.tcx.def_path_str(*definition));
        for definition in auto_traits {
            if !first {
                self.write_str(" + ")?;
            }
            first = false;
            self.print_def_path(definition, &[])?;
        }
        Ok(())
    }

    fn print_const(&mut self, value: ty::Const<'tcx>) -> Result<(), PrintError> {
        self.pretty_print_const(value, false)
    }

    fn print_crate_name(&mut self, krate: CrateNum) -> Result<(), PrintError> {
        self.empty_path = false;
        if krate == LOCAL_CRATE {
            return self.write_str("crate");
        }
        // Cargo 的 --extern 映射包含依赖重命名；直接 extern 还覆盖 rustc 注入的
        // std/core 和用户显式 extern crate。其余 canonical 根只适合作诊断文本。
        self.nameable &= self.crate_names.contains_key(&krate)
            || self
                .tcx
                .extern_crate(krate)
                .is_some_and(|entry| entry.is_direct());
        let name = self
            .crate_names
            .get(&krate)
            .cloned()
            .unwrap_or_else(|| self.tcx.crate_name(krate).to_string());
        let raw = self.identifier_needs_raw(Symbol::intern(&name));
        write!(self, "::{}{name}", if raw { "r#" } else { "" })
    }

    fn print_path_with_simple(
        &mut self,
        prefix: impl FnOnce(&mut Self) -> Result<(), PrintError>,
        data: &DisambiguatedDefPathData,
    ) -> Result<(), PrintError> {
        prefix(self)?;
        if matches!(data.data, DefPathData::ForeignMod | DefPathData::Ctor) {
            return Ok(());
        }
        if !self.empty_path {
            self.write_str("::")?;
        }
        if let DefPathDataName::Named(name) = data.data.name()
            && self.identifier_needs_raw(name)
        {
            self.write_str("r#")?;
        }
        write!(self, "{}", data.as_sym(false))?;
        self.empty_path = false;
        Ok(())
    }

    fn print_path_with_impl(
        &mut self,
        prefix: impl FnOnce(&mut Self) -> Result<(), PrintError>,
        self_ty: Ty<'tcx>,
        trait_ref: Option<ty::TraitRef<'tcx>>,
    ) -> Result<(), PrintError> {
        self.pretty_print_path_with_impl(
            |printer| {
                prefix(printer)?;
                if !printer.empty_path {
                    printer.write_str("::")?;
                }
                Ok(())
            },
            self_ty,
            trait_ref,
        )?;
        self.empty_path = false;
        Ok(())
    }

    fn print_path_with_generic_args(
        &mut self,
        prefix: impl FnOnce(&mut Self) -> Result<(), PrintError>,
        args: &[GenericArg<'tcx>],
    ) -> Result<(), PrintError> {
        prefix(self)?;
        if args.is_empty() {
            return Ok(());
        }
        if self.in_value {
            self.write_str("::")?;
        }
        self.generic_delimiters(|printer| printer.comma_sep(args.iter().copied()))
    }

    fn print_path_with_qualified(
        &mut self,
        self_ty: Ty<'tcx>,
        trait_ref: Option<ty::TraitRef<'tcx>>,
    ) -> Result<(), PrintError> {
        self.pretty_print_path_with_qualified(self_ty, trait_ref)?;
        self.empty_path = false;
        Ok(())
    }

    fn reset_path(&mut self) -> Result<(), PrintError> {
        self.empty_path = true;
        Ok(())
    }
}

impl<'tcx> SourcePrinter<'tcx> {
    fn identifier_needs_raw(&self, name: Symbol) -> bool {
        // Generated overlays are parsed in the consuming crate's edition,
        // even when a name originated in metadata from an older edition.
        name.can_be_raw() && name.is_reserved(|| self.tcx.sess.edition())
    }

    fn print_source_projection(
        &mut self,
        projection: ty::ExistentialProjection<'tcx>,
    ) -> Result<(), PrintError> {
        let name = self.tcx.associated_item(projection.def_id).name();
        let raw = self.identifier_needs_raw(name);
        // Existential args omit Self; the associated item's generics still
        // include it. Keep only the item's own arguments, as rustc does.
        let args = &projection.args[self.tcx.generics_of(projection.def_id).parent_count - 1..];
        self.print_path_with_generic_args(
            |printer| write!(printer, "{}{name}", if raw { "r#" } else { "" }),
            args,
        )?;
        self.write_str(" = ")?;
        projection.term.print(self)
    }
}

impl<'tcx> PrettyPrinter<'tcx> for SourcePrinter<'tcx> {
    fn generic_delimiters(
        &mut self,
        emit: impl FnOnce(&mut Self) -> Result<(), PrintError>,
    ) -> Result<(), PrintError> {
        self.write_str("<")?;
        let in_value = std::mem::replace(&mut self.in_value, false);
        emit(self)?;
        self.in_value = in_value;
        self.write_str(">")
    }

    fn should_print_optional_region(&self, region: ty::Region<'tcx>) -> bool {
        FmtPrinter::new(self.tcx, Namespace::TypeNS).should_print_optional_region(region)
    }

    fn pretty_print_value_path(
        &mut self,
        definition: DefId,
        args: &'tcx [GenericArg<'tcx>],
    ) -> Result<(), PrintError> {
        let in_value = std::mem::replace(&mut self.in_value, true);
        self.print_def_path(definition, args)?;
        self.in_value = in_value;
        Ok(())
    }

    fn pretty_print_in_binder<T>(&mut self, value: &ty::Binder<'tcx, T>) -> Result<(), PrintError>
    where
        T: Print<Self> + TypeFoldable<TyCtxt<'tcx>>,
    {
        self.wrap_binder(value, WrapBinderMode::ForAll, |value, printer| {
            value.print(printer)
        })
    }

    fn wrap_binder<T, F>(
        &mut self,
        value: &ty::Binder<'tcx, T>,
        mode: WrapBinderMode,
        emit: F,
    ) -> Result<(), PrintError>
    where
        T: TypeFoldable<TyCtxt<'tcx>>,
        F: FnOnce(&T, &mut Self) -> Result<(), fmt::Error>,
    {
        // Keep rustc's capture-avoiding lifetime naming while using this
        // printer for the actual typed body and its crate-identity paths.
        let mut names = FmtPrinter::new(self.tcx, Namespace::TypeNS);
        let (named, _) = names.name_all_regions(value, mode)?;
        self.write_str(&names.into_buffer())?;
        emit(&named, self)
    }
}

pub fn accessible(tcx: TyCtxt<'_>, ty: Ty<'_>, module: LocalDefId) -> bool {
    let visible = |mut definition: DefId| {
        loop {
            if !tcx.visibility(definition).is_accessible_from(module, tcx) {
                return false;
            }
            let parent = if definition.is_local() {
                tcx.opt_parent(definition)
            } else {
                tcx.visible_parent_map(())
                    .get(&definition)
                    .copied()
                    .or_else(|| tcx.opt_parent(definition))
            };
            let Some(parent) = parent else {
                return true;
            };
            definition = parent;
            if tcx.def_kind(definition) != DefKind::Mod {
                return false;
            }
        }
    };
    for argument in ty.walk() {
        if let ty::GenericArgKind::Type(nested) = argument.kind() {
            match nested.kind() {
                ty::Adt(definition, _) if !visible(definition.did()) => return false,
                ty::Foreign(definition) if !visible(*definition) => return false,
                ty::Dynamic(predicates, _)
                    if predicates
                        .principal_def_id()
                        .is_some_and(|definition| !visible(definition))
                        || predicates
                            .auto_traits()
                            .any(|definition| !visible(definition)) =>
                {
                    return false;
                }
                _ => {}
            }
        }
    }
    true
}
