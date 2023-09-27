use std::collections::HashMap;

use anyhow::{bail, Result};
use indexmap::IndexMap;
use turbo_tasks::{
    graph::{GraphTraversal, NonDeterministic},
    TryFlatJoinIterExt, ValueToString, Vc,
};
use turbopack_binding::{
    swc::core::ecma::{
        ast::{Callee, Expr, Program, Prop, PropOrSpread, PropName},
        visit::{Visit, VisitWith},
    },
    turbopack::{
        core::{module::Module, reference::primary_referenced_modules},
        ecmascript::{chunk::EcmascriptChunkPlaceable, parse::ParseResult, EcmascriptModuleAsset},
    },
};

pub(crate) async fn create_react_lodable_manifest(
    entry: Vc<Box<dyn EcmascriptChunkPlaceable>>,
) -> Result<()> {
    let all_actions = NonDeterministic::new()
        .skip_duplicates()
        .visit([Vc::upcast(entry)], get_referenced_modules)
        .await
        .completed()?
        .into_inner()
        .into_iter()
        .map(parse_actions_filter_map)
        .try_flat_join()
        .await?
        .into_iter()
        .collect::<IndexMap<_, _>>();

    println!("all_actions: {:?}", all_actions);
    Ok(())
}

async fn get_referenced_modules(
    parent: Vc<Box<dyn Module>>,
) -> Result<impl Iterator<Item = Vc<Box<dyn Module>>> + Send> {
    primary_referenced_modules(parent)
        .await
        .map(|modules| modules.clone_value().into_iter())
}

#[turbo_tasks::function]
async fn parse_imports(module: Vc<Box<dyn Module>>) -> Result<Vc<OptionActionMap>> {
    let Some(ecmascript_asset) =
        Vc::try_resolve_downcast_type::<EcmascriptModuleAsset>(module).await?
    else {
        return Ok(OptionActionMap::none());
    };

    let id = &*module.ident().to_string().await?;
    let ParseResult::Ok { program, .. } = &*ecmascript_asset.parse().await? else {
        bail!("failed to parse module '{id}'");
    };

    let Some(actions) = parse_lodable_imports(id.as_str(), &program) else {
        return Ok(OptionActionMap::none());
    };

    let mut actions = IndexMap::from_iter(actions.into_iter());
    actions.sort_keys();
    Ok(Vc::cell(Some(Vc::cell(actions))))
}

struct LodableImportVisitor {}

impl Visit for LodableImportVisitor {
    fn visit_call_expr(&mut self, call_expr: &turbopack_binding::swc::core::ecma::ast::CallExpr) {
        if let Callee::Import(import) = call_expr.callee {
            println!("import(): {:#?}", call_expr);
        }

        call_expr.visit_children_with(self);
    }

    fn visit_expr_or_spread(
        &mut self,
        expr_or_spread: &turbopack_binding::swc::core::ecma::ast::ExprOrSpread,
    ) {
        let expr = &*expr_or_spread.expr;
        if let Expr::Object(object) = expr {
            let props = object.props.first();
            if let Some(prop) = props {
                if let PropOrSpread::Prop(prop) = &prop {
                    if let Prop::KeyValue(key_value) = &**prop {
                        if let PropName::Ident(ident) = &key_value.key {
                            if ident.sym == *"loadableGenerated" {
                                //packages/next-swc/crates/next-transform-dynamic/tests/fixture/no-options/output-turbo-dev-server.js
                                println!("loadableGenerated: {:#?}", key_value.value);
                            }
                        }
                    }
                }
            }
        }

        expr_or_spread.visit_children_with(self);
    }
}

pub fn parse_lodable_imports(id: &str, program: &Program) -> Option<ActionsMap> {
    let mut visitor = LodableImportVisitor {};

    program.visit_with(&mut visitor);

    println!("{}", id);
    println!("=========================");
    None
}

async fn parse_actions_filter_map(
    module: Vc<Box<dyn Module>>,
) -> Result<Option<(Vc<Box<dyn Module>>, Vc<ActionMap>)>> {
    parse_imports(module).await.map(|option_action_map| {
        option_action_map
            .clone_value()
            .map(|action_map| (module, action_map))
    })
}

pub type ActionsMap = HashMap<String, String>;

#[turbo_tasks::value(transparent)]
struct ActionMap(IndexMap<String, String>);

/// An Option wrapper around [ActionMap].
#[turbo_tasks::value(transparent)]
struct OptionActionMap(Option<Vc<ActionMap>>);

#[turbo_tasks::value_impl]
impl OptionActionMap {
    #[turbo_tasks::function]
    pub fn none() -> Vc<Self> {
        Vc::cell(None)
    }
}

#[turbo_tasks::value(transparent)]
struct ModuleActionMap(IndexMap<Vc<Box<dyn Module>>, Vc<ActionMap>>);

#[turbo_tasks::value_impl]
impl ModuleActionMap {
    #[turbo_tasks::function]
    pub fn empty() -> Vc<Self> {
        Vc::cell(IndexMap::new())
    }
}
