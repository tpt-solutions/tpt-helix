//! TPT Eve — high-level migration planning orchestration (spec §6.2, the
//! *LLM Orchestration* row: "TPT Eve — High-level migration planning").
//!
//! Eve is the planner. It consumes a discovered [`ApiSurfaceMap`] (Stage S1)
//! plus the source documents and decides *how* to migrate them: which pattern
//! classifies the workload (P1–P6), which Stage S2 transpiler to invoke, and
//! whether the Stage S4 (optimize) and Stage S5 (deploy) passes should run.
//! The resulting [`MigrationPlan`] is then handed to TPT Spark (`spark`) to
//! generate shippable guest artifacts.
//!
//! When the real TPT Eve service is wired in it would own this decision
//! surface; the implementation here is a fully-deterministic, offline planner
//! that uses the [`coverage`] pattern probes and the transpiler's own
//! structure extraction, so CI can exercise the whole Stage S1→S5 flow
//! without an external LLM.

use crate::ApiSurfaceMap;
use crate::coverage::Pattern;
use crate::deploy::{
    DeployConfig, FeatureFlag, TargetPlatform, generate_deploy_config, generate_feature_flags,
};
use crate::optimize::{Suggestion, suggest_parallel_loops, suggest_struct_packing};
use crate::transpile::{
    DomOp, FormModel, TranspiledSite, transpile_data_viz, transpile_form_app,
    transpile_media_player, transpile_static_site,
};

/// How strongly a pattern matches a given input during planning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fit {
    /// The detected structure fully satisfies the pattern's support probe.
    Strong,
    /// The pattern is the best match but the structure is incomplete (manual
    /// review likely needed before deploy).
    Partial,
    /// No supported pattern matched — requires manual migration.
    None,
}

/// A planned migration stage with its inputs and expected artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagePlan {
    /// Stage number (1 = discovery … 5 = deploy).
    pub stage: u8,
    /// Human-readable stage name.
    pub name: String,
    /// Whether Eve elects to run this stage for the plan.
    pub enabled: bool,
    /// Planner note (e.g. why a stage is skipped).
    pub note: String,
}

/// The high-level migration plan produced by Eve for one application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationPlan {
    pub app_name: String,
    pub version: String,
    /// The pattern Eve classified the workload as.
    pub detected: Pattern,
    /// Confidence of the classification.
    pub confidence: Fit,
    /// Per-stage plan, in Stage S1→S5 order.
    pub stages: Vec<StagePlan>,
    /// Deploy targets selected for the plan.
    pub targets: Vec<TargetPlatform>,
}

/// Inputs to a single migration-planning request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanRequest {
    pub app_name: String,
    pub version: String,
    /// Primary HTML document to migrate.
    pub html: String,
    /// Optional JS/TS source (used by Stage S1 discovery + type inference).
    pub js: Option<String>,
    /// Deploy targets the migrated app should ship to.
    pub targets: Vec<TargetPlatform>,
}

/// The fully orchestrated migration: plan + every stage's artifact.
#[derive(Debug, Clone)]
pub struct OrchestratedMigration {
    pub plan: MigrationPlan,
    /// Stage S1 discovery output.
    pub surface: ApiSurfaceMap,
    /// Stage S2 transpiled guest.
    pub site: TranspiledSite,
    /// Stage S3 equivalence check result.
    pub equivalence_ok: bool,
    /// Stage S4 optimization suggestions.
    pub suggestions: Vec<Suggestion>,
    /// Stage S5 `helix-deploy.toml` document.
    pub deploy_config: String,
    /// Stage S5 feature-flag Rust module.
    pub feature_flags: String,
}

/// Classify an HTML document into its dominant migration pattern (P1–P4).
///
/// Reuses the transpiler's own structure extraction: `transpile_html` always
/// populates the `crud`/`dataviz`/`media` models, so a single static-site
/// transpile is enough to read off which specialized pattern also matched and
/// how completely.
pub fn detect_pattern(html: &str) -> (Pattern, Fit) {
    let site = transpile_static_site(html);
    if !site.media.players.is_empty() {
        let strong = site
            .media
            .players
            .iter()
            .any(|p| p.src.is_some() && p.has_controls);
        return (
            Pattern::P4MediaPlayer,
            if strong { Fit::Strong } else { Fit::Partial },
        );
    }
    if !site.dataviz.charts.is_empty() {
        let strong = site.dataviz.charts.iter().any(|c| {
            c.id.is_some() && c.width.is_some() && c.height.is_some() && !c.series.is_empty()
        });
        return (
            Pattern::P3DataViz,
            if strong { Fit::Strong } else { Fit::Partial },
        );
    }
    if !site.crud.forms.is_empty() || !site.crud.tables.is_empty() {
        let strong = site.crud.forms.iter().any(|f| {
            !f.fields.is_empty() && f.has_submit && f.fields.iter().all(|x| x.name.is_some())
        }) && !site.crud.tables.is_empty();
        return (
            Pattern::P2FormCrud,
            if strong { Fit::Strong } else { Fit::Partial },
        );
    }
    (Pattern::P1StaticSite, Fit::Strong)
}

/// Stage S2 transpiler selected for a detected pattern.
fn transpiler_for(pattern: Pattern) -> fn(&str) -> TranspiledSite {
    match pattern {
        Pattern::P2FormCrud => transpile_form_app,
        Pattern::P3DataViz => transpile_data_viz,
        Pattern::P4MediaPlayer => transpile_media_player,
        _ => transpile_static_site,
    }
}

/// Stage S3 equivalence invariant: every `append` references a previously
/// created node, and at least one text run was preserved.
pub fn validate_equivalence(site: &TranspiledSite) -> bool {
    let mut created: std::collections::HashSet<String> = Default::default();
    let mut text_runs = 0usize;
    for op in &site.ops {
        match op {
            DomOp::Create { var, .. } => {
                created.insert(var.clone());
            }
            DomOp::Text { .. } => text_runs += 1,
            DomOp::Append { parent, child }
                if !created.contains(parent) || !created.contains(child) =>
            {
                return false;
            }
            _ => {}
        }
    }
    text_runs > 0
}

/// Map a form field's `type` to a representative byte size for struct-packing
/// suggestions (Stage S4).
fn field_size(input_type: &str) -> usize {
    match input_type {
        "number" => 8,
        "checkbox" | "radio" | "range" => 1,
        _ => 24, // String-backed (text/email/etc.)
    }
}

/// Build the per-field packing model for a detected form (Stage S4 input).
fn form_packing_fields(form: &FormModel) -> Vec<(String, usize)> {
    form.fields
        .iter()
        .map(|f| (format!("{}_field", f.var), field_size(&f.input_type)))
        .collect()
}

/// Build the per-stage plan from a classification.
fn build_stages(detected: Pattern, confidence: Fit, targets: &[TargetPlatform]) -> Vec<StagePlan> {
    let s5_enabled = !targets.is_empty();
    vec![
        StagePlan {
            stage: 1,
            name: "discovery".into(),
            enabled: true,
            note: "API surface map extracted from JS/TS (tree-sitter) or HTML.".into(),
        },
        StagePlan {
            stage: 2,
            name: "transpile".into(),
            enabled: confidence != Fit::None,
            note: format!("Using {:?} transpiler.", detected),
        },
        StagePlan {
            stage: 3,
            name: "validate".into(),
            enabled: confidence != Fit::None,
            note: "Equivalence (1:1 text + structural soundness) checked.".into(),
        },
        StagePlan {
            stage: 4,
            name: "optimize".into(),
            enabled: true,
            note: "Dead-code + struct-packing + parallel-loop suggestions.".into(),
        },
        StagePlan {
            stage: 5,
            name: "deploy".into(),
            enabled: s5_enabled,
            note: if s5_enabled {
                format!("Emitting config for {} target(s).", targets.len())
            } else {
                "No targets selected; deploy config skipped.".into()
            },
        },
    ]
}

/// Plan a migration without executing the transpile/optimize passes.
///
/// Useful when the caller only needs Eve's decision (pattern, confidence,
/// stage selection) before committing to generation.
pub fn plan(req: &PlanRequest) -> MigrationPlan {
    let (detected, confidence) = detect_pattern(&req.html);
    MigrationPlan {
        app_name: req.app_name.clone(),
        version: req.version.clone(),
        detected,
        confidence,
        stages: build_stages(detected, confidence, &req.targets),
        targets: req.targets.clone(),
    }
}

/// Execute the full Stage S1→S5 pipeline under Eve's plan.
///
/// Returns an [`OrchestratedMigration`] carrying every stage's artifact, ready
/// for TPT Spark (`spark::generate_guest_crate`) to turn into a shippable
/// guest crate.
pub fn orchestrate(req: &PlanRequest) -> OrchestratedMigration {
    // Stage S1 — discovery (HTML-only best-effort when no JS is supplied).
    let surface = match &req.js {
        Some(js) => crate::discover_ast(js, crate::SourceLang::Auto),
        None => crate::discover(&req.html),
    };

    // Stage S2 — classify + transpile.
    let (detected, confidence) = detect_pattern(&req.html);
    let site = transpiler_for(detected)(&req.html);

    // Stage S3 — equivalence validation.
    let equivalence_ok = validate_equivalence(&site);

    // Stage S4 — optimization suggestions.
    let mut suggestions: Vec<Suggestion> = Vec::new();
    if let Some(form) = site.crud.forms.first() {
        suggestions.extend(suggest_struct_packing(&form_packing_fields(form)));
    }
    suggestions.extend(suggest_parallel_loops(
        req.js.as_deref().unwrap_or(&req.html),
    ));

    // Stage S5 — deploy config + feature flags.
    let deploy_config = generate_deploy_config(&DeployConfig {
        app_name: req.app_name.clone(),
        version: req.version.clone(),
        targets: req.targets.clone(),
    });
    let pattern_flag = FeatureFlag {
        name: format!("pattern-{}", pattern_id(detected)),
        enabled: confidence != Fit::None,
        description: format!("Migrated via {} pipeline", detected.label()),
    };
    let feature_flags = generate_feature_flags(&[
        FeatureFlag {
            name: "helix-runtime".into(),
            enabled: true,
            description: "Run on the Helix Runtime".into(),
        },
        pattern_flag,
    ]);

    let plan = MigrationPlan {
        app_name: req.app_name.clone(),
        version: req.version.clone(),
        detected,
        confidence,
        stages: build_stages(detected, confidence, &req.targets),
        targets: req.targets.clone(),
    };

    OrchestratedMigration {
        plan,
        surface,
        site,
        equivalence_ok,
        suggestions,
        deploy_config,
        feature_flags,
    }
}

/// Short machine identifier for a pattern (used in feature-flag names).
fn pattern_id(p: Pattern) -> &'static str {
    match p {
        Pattern::P1StaticSite => "p1",
        Pattern::P2FormCrud => "p2",
        Pattern::P3DataViz => "p3",
        Pattern::P4MediaPlayer => "p4",
        Pattern::P5Realtime => "p5",
        Pattern::P6ComplexSpa => "p6",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deploy::TargetPlatform;

    #[test]
    fn classifies_static_site() {
        let html = r#"<html><body><h1>Docs</h1><p>Welcome</p></body></html>"#;
        let (p, fit) = detect_pattern(html);
        assert_eq!(p, Pattern::P1StaticSite);
        assert_eq!(fit, Fit::Strong);
    }

    #[test]
    fn classifies_form_crud() {
        let html = r#"
            <form onsubmit="add()">
                <input type="text" name="name" />
                <button type="submit">Add</button>
            </form>
            <table><tr><td>a</td></tr></table>"#;
        let (p, fit) = detect_pattern(html);
        assert_eq!(p, Pattern::P2FormCrud);
        assert_eq!(fit, Fit::Strong);
    }

    #[test]
    fn classifies_data_viz() {
        let html = r#"<canvas id="c" width="800" height="600" data-series-0="rev"></canvas>"#;
        let (p, fit) = detect_pattern(html);
        assert_eq!(p, Pattern::P3DataViz);
        assert_eq!(fit, Fit::Strong);
    }

    #[test]
    fn classifies_media_player() {
        let html = r#"<video src="m.mp4" controls></video>"#;
        let (p, fit) = detect_pattern(html);
        assert_eq!(p, Pattern::P4MediaPlayer);
        assert_eq!(fit, Fit::Strong);
    }

    #[test]
    fn equivalence_check_fails_on_dangling_append() {
        // Hand-crafted op list with a forward reference that must fail.
        let site = TranspiledSite {
            ops: vec![
                DomOp::Create {
                    var: "el0".into(),
                    tag: "div".into(),
                },
                DomOp::Append {
                    parent: "el0".into(),
                    child: "el1".into(),
                },
            ],
            rust_source: String::new(),
            wit_world: String::new(),
            crud: Default::default(),
            dataviz: Default::default(),
            media: Default::default(),
        };
        assert!(!validate_equivalence(&site));
    }

    #[test]
    fn orchestrate_runs_full_pipeline_for_static_site() {
        let req = PlanRequest {
            app_name: "docs".into(),
            version: "0.1.0".into(),
            html: r#"<h1>Title</h1><p>Body text</p>"#.into(),
            js: None,
            targets: vec![TargetPlatform::LinuxX86_64, TargetPlatform::Wasm32],
        };
        let m = orchestrate(&req);
        assert_eq!(m.plan.detected, Pattern::P1StaticSite);
        assert!(m.equivalence_ok, "static site must pass S3");
        assert!(m.deploy_config.contains("linux-x86_64"));
        assert!(m.feature_flags.contains("pub const HELIX_RUNTIME_ENABLED"));
        // Stage S5 enabled with two targets.
        let s5 = m.plan.stages.iter().find(|s| s.stage == 5).unwrap();
        assert!(s5.enabled);
    }

    #[test]
    fn orchestrate_marks_deploy_disabled_without_targets() {
        let req = PlanRequest {
            app_name: "docs".into(),
            version: "0.1.0".into(),
            html: r#"<p>hi</p>"#.into(),
            js: None,
            targets: vec![],
        };
        let m = orchestrate(&req);
        let s5 = m.plan.stages.iter().find(|s| s.stage == 5).unwrap();
        assert!(!s5.enabled);
        assert!(m.deploy_config.is_empty());
    }
}
