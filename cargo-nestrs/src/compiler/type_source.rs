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
    /// 当前会话的类型和可见路径查询入口。
    tcx: TyCtxt<'tcx>,

    /// 实际加载 crate 身份到 Cargo extern 名称的共享映射。
    crate_names: Rc<HashMap<CrateNum, String>>,
}

impl<'tcx> SourceTypes<'tcx> {
    /// 按本轮加载工件建立 extern 别名目录，供所有类型打印复用。
    pub fn new(tcx: TyCtxt<'tcx>) -> Self {
        Self {
            tcx,
            crate_names: Rc::new(extern_names(tcx)),
        }
    }

    /// 返回完整类型文本；生成源码需要另用 render_if_nameable 校验可命名性。
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

    /// 仅为打印副本重命名绑定生命周期，返回源码文本及全部路径是否可命名。
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

/// 为打印副本分配无歧义的绑定生命周期名称，不改变候选类型身份。
struct SourceRegions<'tcx> {
    /// 创建重命名生命周期时使用的类型上下文。
    tcx: TyCtxt<'tcx>,

    /// 当前嵌套 binder 的唯一编号栈。
    binders: Vec<usize>,

    /// 下一个可分配 binder 编号。
    next_binder: usize,
}

/// 由 binder 与变量编号生成互不冲突的打印用生命周期名称。
fn region_name(binder: usize, variable: usize) -> Symbol {
    Symbol::intern(&format!("'__nestrs_{binder}_{variable}"))
}

impl<'tcx> TypeFolder<TyCtxt<'tcx>> for SourceRegions<'tcx> {
    /// 提供类型折叠使用的同一编译上下文。
    fn cx(&self) -> TyCtxt<'tcx> {
        self.tcx
    }

    /// 为当前 binder 的生命周期分配唯一打印名称，并维护嵌套 binder 栈。
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

    /// 按实际绑定层级恢复对应打印名称，保留非绑定生命周期原义。
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

/// 比较已加载工件与 extern prelude，恢复可以写入源码的实际依赖别名。
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

/// 沿 rustc 可见重导出打印可编译的类型路径并追踪可命名性。
struct SourcePrinter<'tcx> {
    /// 当前会话的类型与路径查询入口。
    tcx: TyCtxt<'tcx>,

    /// 累计生成的类型源码。
    output: String,

    /// 当前路径是否尚未打印首段。
    empty_path: bool,

    /// 是否处于需要表达式路径语法的值上下文。
    in_value: bool,

    /// 真实 extern 别名映射，区分同名依赖版本。
    crate_names: Rc<HashMap<CrateNum, String>>,

    /// 已打印的所有 crate 根是否均可从当前源码命名。
    nameable: bool,
}

impl fmt::Write for SourcePrinter<'_> {
    /// 将打印结果累积到内存字符串。
    fn write_str(&mut self, value: &str) -> fmt::Result {
        self.output.push_str(value);
        Ok(())
    }
}

impl<'tcx> Printer<'tcx> for SourcePrinter<'tcx> {
    /// 向 rustc 打印器提供当前会话。
    fn tcx<'a>(&'a self) -> TyCtxt<'tcx> {
        self.tcx
    }

    /// 优先使用真实可见重导出路径，携带泛型实参时沿用原生路径规则。
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

    /// 沿用 rustc 对生命周期的语法打印。
    fn print_region(&mut self, region: ty::Region<'tcx>) -> Result<(), PrintError> {
        self.write_str(&FmtPrinter::print_string(
            self.tcx,
            Namespace::TypeNS,
            |printer| printer.print_region(region),
        )?)
    }

    /// 复用原生类型结构打印，并保留本打印器的路径策略。
    fn print_type(&mut self, ty: Ty<'tcx>) -> Result<(), PrintError> {
        self.pretty_print_type(ty)
    }

    /// 为关键字形式的关联类型名称保留 raw 标识符，其余 dyn 语法沿用 rustc。
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

    /// 复用原生 const 打印并保留当前路径与值上下文。
    fn print_const(&mut self, value: ty::Const<'tcx>) -> Result<(), PrintError> {
        self.pretty_print_const(value, false)
    }

    /// 使用当前 extern 别名打印 crate 根，同时记录无法命名的传递依赖。
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

    /// 打印普通路径段，并为当前 edition 的关键字补 raw 标识符语法。
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

    /// 打印 impl 关联项路径，保留真实 Self 与 trait 关系。
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

    /// 按当前值/类型上下文输出泛型实参及分隔符。
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

    /// 打印带 Self 和 trait 限定的关联路径。
    fn print_path_with_qualified(
        &mut self,
        self_ty: Ty<'tcx>,
        trait_ref: Option<ty::TraitRef<'tcx>>,
    ) -> Result<(), PrintError> {
        self.pretty_print_path_with_qualified(self_ty, trait_ref)?;
        self.empty_path = false;
        Ok(())
    }

    /// 开始独立路径时重置路径前缀状态。
    fn reset_path(&mut self) -> Result<(), PrintError> {
        self.empty_path = true;
        Ok(())
    }
}

impl<'tcx> SourcePrinter<'tcx> {
    /// 按当前 edition 判断路径名称是否需要 raw 标识符。
    fn identifier_needs_raw(&self, name: Symbol) -> bool {
        // Generated overlays are parsed in the consuming crate's edition,
        // even when a name originated in metadata from an older edition.
        name.can_be_raw() && name.is_reserved(|| self.tcx.sess.edition())
    }

    /// 以合法源码标识符输出关联类型约束及其值。
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
    /// 输出泛型尖括号，临时切换到类型上下文打印内部实参。
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

    /// 保留会影响可编译类型表达式的生命周期信息。
    fn should_print_optional_region(&self, region: ty::Region<'tcx>) -> bool {
        FmtPrinter::new(self.tcx, Namespace::TypeNS).should_print_optional_region(region)
    }

    /// 在值路径上下文中打印函数等项目，再恢复原打印状态。
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

    /// 通过统一 binder 包装规则打印受绑定的类型结构。
    fn pretty_print_in_binder<T>(&mut self, value: &ty::Binder<'tcx, T>) -> Result<(), PrintError>
    where
        T: Print<Self> + TypeFoldable<TyCtxt<'tcx>>,
    {
        self.wrap_binder(value, WrapBinderMode::ForAll, |value, printer| {
            value.print(printer)
        })
    }

    /// 借用 rustc 的捕获规避命名，再用本打印器输出实际类型及 extern 路径。
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

/// 按真实可见性与重导出父链检查类型能否在指定模块合法命名。
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
