//! Deterministic local module graph for Motion JSX.
//!
//! Modules are resolved from an explicit, project-root-relative source closure. The linker never
//! reads the filesystem and never performs npm/URL resolution. It rewrites module-scope bindings
//! by OXC symbol identity before handing one closed program to the established Motion lowering
//! pipeline; linked byte ranges are retained so diagnostics and the source map point back to the
//! original file.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Component, Path, PathBuf};

use oxc::allocator::Allocator;
use oxc::ast::ast::{
    Declaration, ExportDefaultDeclarationKind, ImportDeclarationSpecifier, ImportOrExportKind,
    Program, Statement,
};
use oxc::ast_visit::{Visit, walk};
use oxc::codegen::{Codegen, CodegenOptions, CommentOptions};
use oxc::parser::Parser;
use oxc::semantic::{Scoping, SemanticBuilder};
use oxc::span::{GetSpan, SourceType, Span};
use oxc::syntax::symbol::SymbolId;

use valle_motion::diag::DiagCode;
use valle_motion::{ContentDigest, ResourceRef};

use super::{
    CompiledMotion, CompilerDiagnostic, MeasureEnv, ModuleSourceInfo, PrepareDataBinding,
    ShaderRegistryEnv, SourceSpan, compile_motion_with_full_env_and_data,
};

/// Complete, explicit source closure accepted by every Motion host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MotionModuleGraph {
    pub entry: String,
    pub modules: BTreeMap<String, String>,
}

impl MotionModuleGraph {
    pub fn new(
        entry: impl Into<String>,
        modules: BTreeMap<String, String>,
    ) -> Result<Self, Vec<CompilerDiagnostic>> {
        let entry = normalize_project_path(&entry.into()).map_err(|message| {
            vec![path_diagnostic(
                "<module-graph>",
                "",
                Span::new(0, 0),
                message,
            )]
        })?;
        let mut normalized = BTreeMap::new();
        for (path, source) in modules {
            let canonical = normalize_project_path(&path).map_err(|message| {
                vec![path_diagnostic(&path, &source, Span::new(0, 0), message)]
            })?;
            if normalized.insert(canonical.clone(), source).is_some() {
                return Err(vec![path_diagnostic(
                    &canonical,
                    "",
                    Span::new(0, 0),
                    format!("duplicate module path after normalization: `{canonical}`"),
                )]);
            }
        }
        if !normalized.contains_key(&entry) {
            return Err(vec![path_diagnostic(
                &entry,
                "",
                Span::new(0, 0),
                format!("entry module `{entry}` is not present in the source closure"),
            )]);
        }
        Ok(Self {
            entry,
            modules: normalized,
        })
    }

    /// Load exactly the statically reachable closure rooted at one standalone source file.
    /// Unrelated siblings are never scanned, read or fingerprinted.
    pub fn from_entry_path(input: &Path) -> Result<Self, Vec<CompilerDiagnostic>> {
        let root = input.parent().unwrap_or_else(|| Path::new("."));
        let entry = input
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                vec![path_diagnostic(
                    "<module-graph>",
                    "",
                    Span::new(0, 0),
                    "Motion input path must have a UTF-8 file name",
                )]
            })?
            .to_owned();
        let mut modules = BTreeMap::new();
        load_disk_module(root, &entry, &mut modules, &mut Vec::new())?;
        Self::new(entry, modules)
    }
}

pub fn compile_motion_modules(
    graph: &MotionModuleGraph,
) -> Result<CompiledMotion, Vec<CompilerDiagnostic>> {
    compile_motion_modules_with_full_env(graph, &[], None, None)
}

pub fn compile_motion_modules_with_full_env(
    graph: &MotionModuleGraph,
    resources: &[ResourceRef],
    measure: Option<&MeasureEnv>,
    shaders: Option<&ShaderRegistryEnv>,
) -> Result<CompiledMotion, Vec<CompilerDiagnostic>> {
    compile_motion_modules_with_full_env_and_data(graph, resources, measure, shaders, None)
}

pub fn compile_motion_modules_with_full_env_and_data(
    graph: &MotionModuleGraph,
    resources: &[ResourceRef],
    measure: Option<&MeasureEnv>,
    shaders: Option<&ShaderRegistryEnv>,
    data: Option<&PrepareDataBinding>,
) -> Result<CompiledMotion, Vec<CompilerDiagnostic>> {
    let linked = Linker::new(graph).link()?;
    let mut compiled = match compile_motion_with_full_env_and_data(
        &linked.source,
        resources,
        measure,
        shaders,
        data,
    ) {
        Ok(compiled) => compiled,
        Err(mut diagnostics) => {
            for diagnostic in &mut diagnostics {
                linked.remap_diagnostic(diagnostic);
            }
            return Err(diagnostics);
        }
    };

    let closure_digest = ContentDigest::of_bytes(linked.normalized_closure.as_bytes());
    compiled.normalized_source = linked.normalized_closure.clone();
    compiled.normalized_ast_digest = closure_digest;
    compiled.source_map.version = super::MOTION_SOURCE_MAP_VERSION;
    compiled.source_map.entry = graph.entry.clone();
    compiled.source_map.closure_digest = closure_digest;
    compiled.source_map.modules = linked.module_infos.clone();

    if let Some(span) = compiled.source_map.controls {
        let mapped = linked.remap_span(span);
        compiled.source_map.controls_source_path = Some(mapped.path);
        compiled.source_map.controls = Some(mapped.span);
    } else {
        compiled.source_map.controls_source_path = None;
    }
    for mapping in &mut compiled.source_map.nodes {
        let mapped = linked.remap_span(mapping.span);
        mapping.source_path = mapped.path;
        mapping.span = mapped.span;
        linked.remap_expansion_stack(&mut mapping.expansion_stack);
    }
    for mapping in &mut compiled.source_map.exprs {
        let mapped = linked.remap_span(mapping.span);
        mapping.source_path = mapped.path;
        mapping.span = mapped.span;
        linked.remap_expansion_stack(&mut mapping.expansion_stack);
    }
    for mapping in &mut compiled.source_map.objects {
        let mapped = linked.remap_span(mapping.span);
        mapping.source_path = mapped.path;
        mapping.span = mapped.span;
        linked.remap_expansion_stack(&mut mapping.expansion_stack);
    }
    Ok(compiled)
}

struct Linker<'a> {
    graph: &'a MotionModuleGraph,
    next_module_ordinal: usize,
    visiting: Vec<String>,
    emitted: BTreeSet<String>,
    outputs: BTreeMap<String, ModuleOutput>,
    linked_source: String,
    segments: Vec<LinkedSegment>,
}

impl<'a> Linker<'a> {
    fn new(graph: &'a MotionModuleGraph) -> Self {
        Self {
            graph,
            // Only entry-reachable modules receive a linker-local namespace. The ordinal is
            // deterministic DFS discovery state, not a content identity or fingerprint.
            next_module_ordinal: 0,
            visiting: Vec::new(),
            emitted: BTreeSet::new(),
            outputs: BTreeMap::new(),
            linked_source: String::new(),
            segments: Vec::new(),
        }
    }

    fn link(mut self) -> Result<LinkedSource<'a>, Vec<CompilerDiagnostic>> {
        self.visit(&self.graph.entry)?;
        let mut module_infos = self
            .outputs
            .values()
            .map(|output| output.info.clone())
            .collect::<Vec<_>>();
        module_infos.sort_by(|left, right| left.path.cmp(&right.path));
        let normalized_closure = module_infos
            .iter()
            .map(|info| {
                let output = &self.outputs[&info.path];
                format!("// module:{}\n{}\n", info.path, output.normalized_ast)
            })
            .collect::<String>();
        let display_names = self
            .outputs
            .values()
            .flat_map(|output| output.display_names.clone())
            .collect();
        Ok(LinkedSource {
            graph: self.graph,
            source: self.linked_source,
            segments: self.segments,
            module_infos,
            normalized_closure,
            display_names,
        })
    }

    fn visit(&mut self, path: &str) -> Result<(), Vec<CompilerDiagnostic>> {
        if self.emitted.contains(path) {
            return Ok(());
        }
        if let Some(index) = self.visiting.iter().position(|item| item == path) {
            let mut cycle = self.visiting[index..].to_vec();
            cycle.push(path.to_owned());
            let source = self.graph.modules.get(path).map_or("", String::as_str);
            return Err(vec![path_diagnostic(
                path,
                source,
                Span::new(0, 0),
                format!("Motion module cycle is forbidden: {}", cycle.join(" -> ")),
            )]);
        }
        let module_ordinal = if path == self.graph.entry {
            None
        } else {
            let ordinal = self.next_module_ordinal;
            self.next_module_ordinal += 1;
            Some(ordinal)
        };
        self.visiting.push(path.to_owned());
        let source = &self.graph.modules[path];
        let dependencies = discover_dependencies(path, source, self.graph, &self.visiting)?;
        for dependency in &dependencies {
            self.visit(&dependency.path)?;
        }
        let output = analyze_module(
            path,
            source,
            module_ordinal,
            &dependencies,
            &self.outputs,
            &self.visiting,
        )?;
        self.append_module(&output);
        self.outputs.insert(path.to_owned(), output);
        self.emitted.insert(path.to_owned());
        self.visiting.pop();
        Ok(())
    }

    fn append_module(&mut self, output: &ModuleOutput) {
        for chunk in &output.chunks {
            if !self.linked_source.is_empty() && !self.linked_source.ends_with('\n') {
                self.linked_source.push('\n');
            }
            let linked_start = self.linked_source.len() as u32;
            self.linked_source.push_str(&chunk.text);
            let linked_end = self.linked_source.len() as u32;
            self.segments.push(LinkedSegment {
                linked_start,
                linked_end,
                path: output.info.path.clone(),
                original_start: chunk.original.start,
                original_end: chunk.original.end,
                exact: chunk.exact,
            });
        }
        self.linked_source.push('\n');
    }
}

#[derive(Clone)]
struct Dependency {
    path: String,
}

struct ModuleOutput {
    info: ModuleSourceInfo,
    normalized_ast: String,
    exports: BTreeMap<String, String>,
    display_names: BTreeMap<String, String>,
    chunks: Vec<SourceChunk>,
}

struct SourceChunk {
    text: String,
    original: Span,
    exact: bool,
}

struct LinkedSegment {
    linked_start: u32,
    linked_end: u32,
    path: String,
    original_start: u32,
    original_end: u32,
    exact: bool,
}

struct MappedSpan {
    path: String,
    span: SourceSpan,
}

struct LinkedSource<'a> {
    graph: &'a MotionModuleGraph,
    source: String,
    segments: Vec<LinkedSegment>,
    module_infos: Vec<ModuleSourceInfo>,
    normalized_closure: String,
    display_names: BTreeMap<String, String>,
}

impl LinkedSource<'_> {
    fn remap_expansion_stack(&self, stack: &mut [String]) {
        for name in stack {
            if let Some(display) = self.display_names.get(name) {
                *name = display.clone();
            }
        }
    }

    fn remap_diagnostic(&self, diagnostic: &mut CompilerDiagnostic) {
        let mapped = self.remap_span(diagnostic.span);
        diagnostic.source_path = Some(mapped.path);
        diagnostic.span = mapped.span;
    }

    fn remap_span(&self, linked: SourceSpan) -> MappedSpan {
        let start = self.map_offset(linked.start, false);
        let end = self.map_offset(linked.end, true);
        let (path, start, end) = match (start, end) {
            (Some((start_path, start)), Some((end_path, end))) if start_path == end_path => {
                (start_path, start, end)
            }
            (Some((path, start)), _) => (path, start, start),
            (_, Some((path, end))) => (path, end, end),
            _ => (self.graph.entry.clone(), 0, 0),
        };
        let source = &self.graph.modules[&path];
        MappedSpan {
            path,
            span: source_span(source, Span::new(start, end)),
        }
    }

    fn map_offset(&self, offset: u32, end: bool) -> Option<(String, u32)> {
        let probe = if end && offset > 0 {
            offset - 1
        } else {
            offset
        };
        let segment = self
            .segments
            .iter()
            .find(|segment| probe >= segment.linked_start && probe < segment.linked_end)?;
        let original = if segment.exact {
            let delta = offset
                .saturating_sub(segment.linked_start)
                .min(segment.original_end - segment.original_start);
            segment.original_start + delta
        } else if end {
            segment.original_end
        } else {
            segment.original_start
        };
        Some((segment.path.clone(), original))
    }
}

fn discover_dependencies(
    path: &str,
    source: &str,
    graph: &MotionModuleGraph,
    chain: &[String],
) -> Result<Vec<Dependency>, Vec<CompilerDiagnostic>> {
    with_program(path, source, |program| {
        #[derive(Default)]
        struct DynamicImportScan {
            span: Option<Span>,
        }
        impl<'a> Visit<'a> for DynamicImportScan {
            fn visit_import_expression(
                &mut self,
                expression: &oxc::ast::ast::ImportExpression<'a>,
            ) {
                if self.span.is_none() {
                    self.span = Some(expression.span);
                }
                walk::walk_import_expression(self, expression);
            }
        }
        let mut dynamic = DynamicImportScan::default();
        dynamic.visit_program(program);
        if let Some(span) = dynamic.span {
            return Err(vec![path_diagnostic(
                path,
                source,
                span,
                "dynamic import() is forbidden; the complete Motion source closure must be known before compilation",
            )]);
        }
        let mut dependencies = Vec::new();
        for statement in &program.body {
            let dependency = match statement {
                Statement::ImportDeclaration(import) => {
                    Some((import.source.value.as_str(), import.span))
                }
                Statement::ExportFromDeclaration(export) => {
                    Some((export.source.value.as_str(), export.span))
                }
                Statement::ExportAllDeclaration(export) => {
                    Some((export.source.value.as_str(), export.span))
                }
                _ => None,
            };
            let Some((specifier, span)) = dependency else {
                continue;
            };
            let resolved = resolve_specifier(path, specifier, graph).map_err(|message| {
                vec![path_diagnostic(
                    path,
                    source,
                    span,
                    format!("{message}; import chain: {}", chain.join(" -> ")),
                )]
            })?;
            dependencies.push(Dependency { path: resolved });
        }
        Ok(dependencies)
    })
}

fn load_disk_module(
    root: &Path,
    module: &str,
    modules: &mut BTreeMap<String, String>,
    chain: &mut Vec<String>,
) -> Result<(), Vec<CompilerDiagnostic>> {
    if modules.contains_key(module) {
        return Ok(());
    }
    if let Some(index) = chain.iter().position(|item| item == module) {
        let mut cycle = chain[index..].to_vec();
        cycle.push(module.to_owned());
        return Err(vec![path_diagnostic(
            module,
            "",
            Span::new(0, 0),
            format!("Motion module cycle is forbidden: {}", cycle.join(" -> ")),
        )]);
    }
    let disk_path = root.join(module);
    let metadata = std::fs::symlink_metadata(&disk_path).map_err(|error| {
        vec![path_diagnostic(
            module,
            "",
            Span::new(0, 0),
            format!("cannot read Motion module `{module}`: {error}"),
        )]
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(vec![path_diagnostic(
            module,
            "",
            Span::new(0, 0),
            format!("Motion module `{module}` must be a regular file inside the project root"),
        )]);
    }
    let source = std::fs::read_to_string(&disk_path).map_err(|error| {
        vec![path_diagnostic(
            module,
            "",
            Span::new(0, 0),
            format!("cannot read Motion module `{module}`: {error}"),
        )]
    })?;
    chain.push(module.to_owned());
    let specifiers = static_dependency_specifiers(module, &source)?;
    for (specifier, span) in specifiers {
        let dependency = resolve_disk_specifier(root, module, &specifier).map_err(|message| {
            vec![path_diagnostic(
                module,
                &source,
                span,
                format!("{message}; import chain: {}", chain.join(" -> ")),
            )]
        })?;
        load_disk_module(root, &dependency, modules, chain)?;
    }
    chain.pop();
    modules.insert(module.to_owned(), source);
    Ok(())
}

fn static_dependency_specifiers(
    path: &str,
    source: &str,
) -> Result<Vec<(String, Span)>, Vec<CompilerDiagnostic>> {
    with_program(path, source, |program| {
        let mut dependencies = Vec::new();
        for statement in &program.body {
            let dependency = match statement {
                Statement::ImportDeclaration(import) => {
                    Some((import.source.value.to_string(), import.span))
                }
                Statement::ExportFromDeclaration(export) => {
                    Some((export.source.value.to_string(), export.span))
                }
                Statement::ExportAllDeclaration(export) => {
                    Some((export.source.value.to_string(), export.span))
                }
                _ => None,
            };
            if let Some(dependency) = dependency {
                dependencies.push(dependency);
            }
        }
        Ok(dependencies)
    })
}

fn resolve_disk_specifier(root: &Path, importer: &str, specifier: &str) -> Result<String, String> {
    if !specifier.starts_with("./") && !specifier.starts_with("../") {
        return Err(format!(
            "module specifier `{specifier}` is not relative; npm, URL, absolute and host resolution are forbidden"
        ));
    }
    let base = Path::new(importer)
        .parent()
        .unwrap_or_else(|| Path::new(""));
    let normalized = normalize_path_buf(&base.join(specifier))?;
    let candidates = module_candidates(&normalized);
    let matches = candidates
        .into_iter()
        .filter(|candidate| exact_regular_file(root, candidate))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [found] => Ok(found.clone()),
        [] => Err(format!(
            "cannot resolve `{specifier}` from `{importer}` inside the supplied project closure"
        )),
        many => Err(format!(
            "ambiguous module `{specifier}` from `{importer}`: {}",
            many.join(", ")
        )),
    }
}

fn exact_regular_file(root: &Path, relative: &str) -> bool {
    let mut current = root.to_path_buf();
    for part in relative.split('/') {
        let Ok(entries) = std::fs::read_dir(&current) else {
            return false;
        };
        let Some(entry) = entries
            .flatten()
            .find(|entry| entry.file_name() == std::ffi::OsStr::new(part))
        else {
            return false;
        };
        current = entry.path();
    }
    std::fs::symlink_metadata(current)
        .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
}

fn analyze_module(
    path: &str,
    source: &str,
    module_ordinal: Option<usize>,
    dependencies: &[Dependency],
    outputs: &BTreeMap<String, ModuleOutput>,
    chain: &[String],
) -> Result<ModuleOutput, Vec<CompilerDiagnostic>> {
    with_program(path, source, |program| {
        let is_entry = module_ordinal.is_none();
        let semantic = SemanticBuilder::new_compiler().build(program);
        if !semantic.diagnostics.is_empty() {
            return Err(semantic
                .diagnostics
                .into_iter()
                .map(|diagnostic| {
                    let span = diagnostic
                        .labels
                        .first()
                        .map_or(Span::new(0, 0), |label| label.span());
                    path_diagnostic(path, source, span, diagnostic.to_string())
                })
                .collect());
        }
        let scoping = semantic.semantic.scoping();
        let root = scoping.root_scope_id();
        let dependency_bindings = outputs
            .values()
            .flat_map(|output| output.display_names.keys().map(String::as_str))
            .collect::<BTreeSet<_>>();
        let mut renamed = HashMap::<SymbolId, String>::new();
        let mut local_names = BTreeMap::<String, String>::new();
        for symbol in scoping.symbol_ids() {
            if scoping.symbol_scope_id(symbol) != root {
                continue;
            }
            let original = scoping.symbol_name(symbol).to_owned();
            if is_entry && dependency_bindings.contains(original.as_str()) {
                return Err(vec![path_diagnostic(
                    path,
                    source,
                    scoping.symbol_span(symbol),
                    format!(
                        "entry binding `{original}` collides with a compiler-owned linked module binding"
                    ),
                )]);
            }
            let target = if is_entry {
                original.clone()
            } else {
                linked_module_binding(
                    module_ordinal.expect("checked non-entry ordinal"),
                    &original,
                )
            };
            renamed.insert(symbol, target.clone());
            local_names.insert(original, target);
        }

        let mut dependency_cursor = 0usize;
        for statement in &program.body {
            let Statement::ImportDeclaration(import) = statement else {
                if matches!(
                    statement,
                    Statement::ExportFromDeclaration(_) | Statement::ExportAllDeclaration(_)
                ) {
                    dependency_cursor += 1;
                }
                continue;
            };
            let dependency = &dependencies[dependency_cursor];
            dependency_cursor += 1;
            if import.phase.is_some() || import.with_clause.is_some() {
                return Err(vec![path_diagnostic(
                    path,
                    source,
                    import.span,
                    "import phase/attributes are outside the deterministic Motion module contract",
                )]);
            }
            if import.import_kind == ImportOrExportKind::Type {
                continue;
            }
            let target = &outputs[&dependency.path];
            let Some(specifiers) = &import.specifiers else {
                return Err(vec![path_diagnostic(
                    path,
                    source,
                    import.span,
                    "side-effect imports are forbidden; Motion modules must export explicit pure values",
                )]);
            };
            for specifier in specifiers {
                let (local, imported, type_only) = match specifier {
                    ImportDeclarationSpecifier::ImportSpecifier(specifier) => (
                        &specifier.local,
                        specifier.imported.name().to_string(),
                        specifier.import_kind == ImportOrExportKind::Type,
                    ),
                    ImportDeclarationSpecifier::ImportDefaultSpecifier(specifier) => {
                        (&specifier.local, "default".to_owned(), false)
                    }
                    ImportDeclarationSpecifier::ImportNamespaceSpecifier(specifier) => {
                        return Err(vec![path_diagnostic(
                            path,
                            source,
                            specifier.span,
                            "namespace imports are not admitted; import explicit named values",
                        )]);
                    }
                };
                if type_only {
                    continue;
                }
                let Some(target_name) = target.exports.get(&imported) else {
                    return Err(vec![path_diagnostic(
                        path,
                        source,
                        local.span,
                        format!(
                            "module `{}` has no value export `{imported}`; import chain: {}",
                            dependency.path,
                            chain.join(" -> ")
                        ),
                    )]);
                };
                let symbol = local
                    .symbol_id
                    .get()
                    .expect("semantic assigns import symbol");
                renamed.insert(symbol, target_name.clone());
                local_names.insert(local.name.to_string(), target_name.clone());
            }
        }

        let mut edits = RenameCollector {
            scoping,
            renamed: &renamed,
            edits: Vec::new(),
        };
        edits.visit_program(program);
        let mut erasures = TypeEraseCollector {
            source,
            edits: Vec::new(),
        };
        erasures.visit_program(program);
        edits.edits.extend(erasures.edits);
        edits.edits.sort_by_key(|edit| edit.span.start);
        edits
            .edits
            .dedup_by_key(|edit| (edit.span.start, edit.span.end));
        let erased = edits
            .edits
            .iter()
            .filter(|edit| edit.replacement.is_empty())
            .map(|edit| edit.span)
            .collect::<Vec<_>>();
        edits.edits.retain(|edit| {
            edit.replacement.is_empty()
                || !erased
                    .iter()
                    .any(|span| edit.span.start >= span.start && edit.span.end <= span.end)
        });

        let mut exports = BTreeMap::new();
        let mut chunks = Vec::new();
        let mut dependency_cursor = 0usize;
        for statement in &program.body {
            match statement {
                Statement::ImportDeclaration(_) => dependency_cursor += 1,
                Statement::ExportDeclaration(export) => {
                    if is_type_declaration(&export.declaration) {
                        continue;
                    }
                    for name in declaration_root_names(&export.declaration, scoping) {
                        insert_export(
                            path,
                            source,
                            export.span,
                            &mut exports,
                            name.clone(),
                            local_names[&name].clone(),
                        )?;
                    }
                    let span = if is_entry {
                        statement.span()
                    } else {
                        export.declaration.span()
                    };
                    chunks.extend(render_span(source, span, &edits.edits));
                }
                Statement::ExportDefaultDeclaration(export) => match &export.declaration {
                    ExportDefaultDeclarationKind::FunctionDeclaration(function) => {
                        let Some(id) = &function.id else {
                            return Err(vec![path_diagnostic(
                                path,
                                source,
                                export.span,
                                "default exported Motion functions must be named",
                            )]);
                        };
                        insert_export(
                            path,
                            source,
                            export.span,
                            &mut exports,
                            "default".into(),
                            local_names[id.name.as_str()].clone(),
                        )?;
                        let span = if is_entry {
                            statement.span()
                        } else {
                            function.span()
                        };
                        chunks.extend(render_span(source, span, &edits.edits));
                    }
                    ExportDefaultDeclarationKind::TSInterfaceDeclaration(_) => {}
                    _ => {
                        return Err(vec![path_diagnostic(
                            path,
                            source,
                            export.span,
                            "default exports must be named functions",
                        )]);
                    }
                },
                Statement::ExportNamedDeclaration(export) => {
                    if export.export_kind == ImportOrExportKind::Type {
                        continue;
                    }
                    for specifier in &export.specifiers {
                        if specifier.export_kind == ImportOrExportKind::Type {
                            continue;
                        }
                        let local = specifier.local.name().to_string();
                        let exported = specifier.exported.name().to_string();
                        let Some(target) = local_names.get(&local) else {
                            return Err(vec![path_diagnostic(
                                path,
                                source,
                                specifier.span,
                                format!("cannot export missing local binding `{local}`"),
                            )]);
                        };
                        insert_export(
                            path,
                            source,
                            specifier.span,
                            &mut exports,
                            exported,
                            target.clone(),
                        )?;
                    }
                }
                Statement::ExportFromDeclaration(export) => {
                    let dependency = &dependencies[dependency_cursor];
                    dependency_cursor += 1;
                    if export.export_kind == ImportOrExportKind::Type {
                        continue;
                    }
                    let target = &outputs[&dependency.path];
                    for specifier in &export.specifiers {
                        if specifier.export_kind == ImportOrExportKind::Type {
                            continue;
                        }
                        let imported = specifier.local.name().to_string();
                        let exported = specifier.exported.name().to_string();
                        let Some(target_name) = target.exports.get(&imported) else {
                            return Err(vec![path_diagnostic(
                                path,
                                source,
                                specifier.span,
                                format!(
                                    "module `{}` has no value export `{imported}`",
                                    dependency.path
                                ),
                            )]);
                        };
                        insert_export(
                            path,
                            source,
                            specifier.span,
                            &mut exports,
                            exported,
                            target_name.clone(),
                        )?;
                    }
                }
                Statement::ExportAllDeclaration(export) => {
                    let dependency = &dependencies[dependency_cursor];
                    dependency_cursor += 1;
                    if export.export_kind == ImportOrExportKind::Type {
                        continue;
                    }
                    if export.exported.is_some() {
                        return Err(vec![path_diagnostic(
                            path,
                            source,
                            export.span,
                            "namespace re-exports are not admitted; re-export explicit names",
                        )]);
                    }
                    for (name, target) in &outputs[&dependency.path].exports {
                        if name != "default" {
                            insert_export(
                                path,
                                source,
                                export.span,
                                &mut exports,
                                name.clone(),
                                target.clone(),
                            )?;
                        }
                    }
                }
                declaration if is_type_statement(declaration) => {}
                other => chunks.extend(render_span(source, other.span(), &edits.edits)),
            }
        }

        let normalized_ast = Codegen::new()
            .with_options(CodegenOptions {
                minify: false,
                comments: CommentOptions {
                    normal: false,
                    jsdoc: false,
                    annotation: false,
                    legal: oxc::codegen::LegalComment::None,
                },
                ..CodegenOptions::default()
            })
            .build(program)
            .code;
        Ok(ModuleOutput {
            info: ModuleSourceInfo {
                path: path.to_owned(),
                source_digest: ContentDigest::of_bytes(source.as_bytes()),
                normalized_ast_digest: ContentDigest::of_bytes(normalized_ast.as_bytes()),
            },
            normalized_ast,
            exports,
            display_names: local_names
                .iter()
                .filter_map(|(original, linked)| {
                    (original != linked).then(|| (linked.clone(), original.clone()))
                })
                .collect(),
            chunks,
        })
    })
}

struct RenameEdit {
    span: Span,
    replacement: String,
}

struct RenameCollector<'a> {
    scoping: &'a Scoping,
    renamed: &'a HashMap<SymbolId, String>,
    edits: Vec<RenameEdit>,
}

impl<'a> Visit<'a> for RenameCollector<'_> {
    fn visit_binding_identifier(&mut self, identifier: &oxc::ast::ast::BindingIdentifier<'a>) {
        if let Some(symbol) = identifier.symbol_id.get()
            && let Some(replacement) = self.renamed.get(&symbol)
            && replacement != identifier.name.as_str()
        {
            self.edits.push(RenameEdit {
                span: identifier.span,
                replacement: replacement.clone(),
            });
        }
        walk::walk_binding_identifier(self, identifier);
    }

    fn visit_identifier_reference(&mut self, identifier: &oxc::ast::ast::IdentifierReference<'a>) {
        if let Some(reference) = identifier.reference_id.get()
            && let Some(symbol) = self.scoping.get_reference(reference).symbol_id()
            && let Some(replacement) = self.renamed.get(&symbol)
            && replacement != identifier.name.as_str()
        {
            self.edits.push(RenameEdit {
                span: identifier.span,
                replacement: replacement.clone(),
            });
        }
        walk::walk_identifier_reference(self, identifier);
    }
}

struct TypeEraseCollector<'a> {
    source: &'a str,
    edits: Vec<RenameEdit>,
}

impl TypeEraseCollector<'_> {
    fn erase(&mut self, span: Span) {
        if span.start < span.end {
            self.edits.push(RenameEdit {
                span,
                replacement: String::new(),
            });
        }
    }

    fn erase_suffix(&mut self, expression: &oxc::ast::ast::Expression<'_>, span: Span) {
        self.erase(Span::new(expression.span().end, span.end));
    }
}

impl<'a> Visit<'a> for TypeEraseCollector<'_> {
    fn visit_ts_type_annotation(&mut self, annotation: &oxc::ast::ast::TSTypeAnnotation<'a>) {
        self.erase(annotation.span);
    }

    fn visit_ts_type_parameter_declaration(
        &mut self,
        declaration: &oxc::ast::ast::TSTypeParameterDeclaration<'a>,
    ) {
        self.erase(declaration.span);
    }

    fn visit_ts_type_parameter_instantiation(
        &mut self,
        instantiation: &oxc::ast::ast::TSTypeParameterInstantiation<'a>,
    ) {
        self.erase(instantiation.span);
    }

    fn visit_ts_as_expression(&mut self, expression: &oxc::ast::ast::TSAsExpression<'a>) {
        self.erase_suffix(&expression.expression, expression.span);
        self.visit_expression(&expression.expression);
    }

    fn visit_ts_satisfies_expression(
        &mut self,
        expression: &oxc::ast::ast::TSSatisfiesExpression<'a>,
    ) {
        self.erase_suffix(&expression.expression, expression.span);
        self.visit_expression(&expression.expression);
    }

    fn visit_ts_non_null_expression(
        &mut self,
        expression: &oxc::ast::ast::TSNonNullExpression<'a>,
    ) {
        self.erase_suffix(&expression.expression, expression.span);
        self.visit_expression(&expression.expression);
    }

    fn visit_ts_instantiation_expression(
        &mut self,
        expression: &oxc::ast::ast::TSInstantiationExpression<'a>,
    ) {
        self.erase(expression.type_arguments.span);
        self.visit_expression(&expression.expression);
    }

    fn visit_formal_parameter(&mut self, parameter: &oxc::ast::ast::FormalParameter<'a>) {
        if parameter.optional {
            let start = parameter.pattern.span().end as usize;
            let end = parameter
                .type_annotation
                .as_ref()
                .map_or(parameter.span.end, |annotation| annotation.span.start)
                as usize;
            if let Some(offset) = self.source[start..end].find('?') {
                self.erase(Span::new(
                    (start + offset) as u32,
                    (start + offset + 1) as u32,
                ));
            }
        }
        walk::walk_formal_parameter(self, parameter);
    }
}

fn render_span(source: &str, span: Span, edits: &[RenameEdit]) -> Vec<SourceChunk> {
    let relevant = edits
        .iter()
        .filter(|edit| edit.span.start >= span.start && edit.span.end <= span.end)
        .collect::<Vec<_>>();
    let mut chunks = Vec::new();
    let mut cursor = span.start;
    for edit in relevant {
        if edit.span.start > cursor {
            chunks.push(SourceChunk {
                text: source[cursor as usize..edit.span.start as usize].to_owned(),
                original: Span::new(cursor, edit.span.start),
                exact: true,
            });
        }
        chunks.push(SourceChunk {
            text: edit.replacement.clone(),
            original: edit.span,
            exact: edit.replacement.len() as u32 == edit.span.size(),
        });
        cursor = edit.span.end;
    }
    if cursor < span.end {
        chunks.push(SourceChunk {
            text: source[cursor as usize..span.end as usize].to_owned(),
            original: Span::new(cursor, span.end),
            exact: true,
        });
    }
    chunks
}

fn declaration_root_names(declaration: &Declaration<'_>, scoping: &Scoping) -> Vec<String> {
    let span = declaration.span();
    scoping
        .symbol_ids()
        .filter(|symbol| {
            scoping.symbol_scope_id(*symbol) == scoping.root_scope_id() && {
                let symbol_span = scoping.symbol_span(*symbol);
                symbol_span.start >= span.start && symbol_span.end <= span.end
            }
        })
        .map(|symbol| scoping.symbol_name(symbol).to_owned())
        .collect()
}

fn insert_export(
    path: &str,
    source: &str,
    span: Span,
    exports: &mut BTreeMap<String, String>,
    name: String,
    target: String,
) -> Result<(), Vec<CompilerDiagnostic>> {
    if let Some(previous) = exports.insert(name.clone(), target.clone())
        && previous != target
    {
        return Err(vec![path_diagnostic(
            path,
            source,
            span,
            format!("duplicate value export `{name}`"),
        )]);
    }
    Ok(())
}

fn is_type_declaration(declaration: &Declaration<'_>) -> bool {
    matches!(
        declaration,
        Declaration::TSTypeAliasDeclaration(_)
            | Declaration::TSInterfaceDeclaration(_)
            | Declaration::TSExternalModuleDeclaration(_)
            | Declaration::TSNamespaceDeclaration(_)
            | Declaration::TSGlobalDeclaration(_)
            | Declaration::TSImportEqualsDeclaration(_)
    )
}

fn is_type_statement(statement: &Statement<'_>) -> bool {
    matches!(
        statement,
        Statement::TSTypeAliasDeclaration(_)
            | Statement::TSInterfaceDeclaration(_)
            | Statement::TSExternalModuleDeclaration(_)
            | Statement::TSNamespaceDeclaration(_)
            | Statement::TSGlobalDeclaration(_)
            | Statement::TSImportEqualsDeclaration(_)
    )
}

fn with_program<T>(
    path: &str,
    source: &str,
    callback: impl for<'a> FnOnce(&'a Program<'a>) -> Result<T, Vec<CompilerDiagnostic>>,
) -> Result<T, Vec<CompilerDiagnostic>> {
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, SourceType::tsx()).parse();
    if !parsed.diagnostics.is_empty() {
        return Err(parsed
            .diagnostics
            .into_iter()
            .map(|diagnostic| {
                let span = diagnostic
                    .labels
                    .first()
                    .map_or(Span::new(0, 0), |label| label.span());
                path_diagnostic(path, source, span, diagnostic.to_string())
            })
            .collect());
    }
    callback(&parsed.program)
}

fn resolve_specifier(
    importer: &str,
    specifier: &str,
    graph: &MotionModuleGraph,
) -> Result<String, String> {
    if !specifier.starts_with("./") && !specifier.starts_with("../") {
        return Err(format!(
            "module specifier `{specifier}` is not relative; npm, URL, absolute and host resolution are forbidden"
        ));
    }
    let base = Path::new(importer)
        .parent()
        .unwrap_or_else(|| Path::new(""));
    let joined = base.join(specifier);
    let normalized = normalize_path_buf(&joined)?;
    let candidates = module_candidates(&normalized);
    let matches = candidates
        .into_iter()
        .filter(|candidate| graph.modules.contains_key(candidate))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [found] => Ok(found.clone()),
        [] => Err(format!(
            "cannot resolve `{specifier}` from `{importer}` inside the supplied project closure"
        )),
        many => Err(format!(
            "ambiguous module `{specifier}` from `{importer}`: {}",
            many.join(", ")
        )),
    }
}

fn module_candidates(normalized: &str) -> [String; 9] {
    [
        normalized.to_owned(),
        format!("{normalized}.ts"),
        format!("{normalized}.tsx"),
        format!("{normalized}.motion.ts"),
        format!("{normalized}.motion.tsx"),
        format!("{normalized}/index.ts"),
        format!("{normalized}/index.tsx"),
        format!("{normalized}/index.motion.ts"),
        format!("{normalized}/index.motion.tsx"),
    ]
}

fn normalize_project_path(path: &str) -> Result<String, String> {
    if path.is_empty() {
        return Err("module path must not be empty".into());
    }
    if path.contains('\\') {
        return Err(format!("module path `{path}` must use `/` separators"));
    }
    normalize_path_buf(Path::new(path))
}

fn normalize_path_buf(path: &Path) -> Result<String, String> {
    let mut output = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => output.push(value),
            Component::CurDir => {}
            Component::ParentDir => {
                if !output.pop() {
                    return Err(format!(
                        "module path `{}` escapes the allowed project root",
                        path.display()
                    ));
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "absolute module path `{}` is outside the deterministic project closure",
                    path.display()
                ));
            }
        }
    }
    output
        .to_str()
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("module path `{}` is empty or non-UTF-8", path.display()))
}

fn linked_module_binding(module_ordinal: usize, original: &str) -> String {
    format!("__valle_m{module_ordinal}_{original}")
}

fn path_diagnostic(
    path: &str,
    source: &str,
    span: Span,
    message: impl Into<String>,
) -> CompilerDiagnostic {
    CompilerDiagnostic {
        class: DiagCode::ModuleShape.class(),
        code: DiagCode::ModuleShape,
        span: source_span(source, span),
        source_path: Some(path.to_owned()),
        message: message.into(),
    }
}

fn source_span(source: &str, span: Span) -> SourceSpan {
    let start = (span.start as usize).min(source.len());
    let end = (span.end as usize).min(source.len()).max(start);
    let prefix = &source[..start];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() as u32 + 1;
    let column = prefix
        .rsplit_once('\n')
        .map_or(prefix, |(_, tail)| tail)
        .chars()
        .count() as u32
        + 1;
    SourceSpan {
        start: start as u32,
        end: end as u32,
        line,
        column,
    }
}
