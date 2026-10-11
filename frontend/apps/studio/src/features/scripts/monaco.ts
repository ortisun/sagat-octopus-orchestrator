/**
 * The only module that imports Monaco. It is loaded on demand (the editor page asks for it through
 * `editorModule.ts`), so the first download of the Studio does not carry the editor; it is bundled
 * with the application and loads its workers from the bundle, because the page may talk to its own
 * origin and nothing else.
 *
 * It offers the rest of the Studio a small facade (`createEditor`) so that no other file has to know
 * how Monaco is driven, and so the component around it can be tested with a fake.
 */
import * as monaco from 'monaco-editor/editor/editor.api';
// The editor's features (find, suggestions, hover, folding, the command palette, the Tab-focus toggle...),
// without the languages; only the ones the Studio needs are registered below, and each loads its
// tokenizer when a model of that language exists. The CSS and HTML services, the language-server
// client and the other eighty languages are not part of the bundle.
import 'monaco-editor/features/register.all';
import 'monaco-editor/languages/definitions/csharp/register';
import 'monaco-editor/languages/definitions/python/register';
import 'monaco-editor/languages/definitions/typescript/register';
import { jsonDefaults } from 'monaco-editor/languages/features/json/register';
import { ModuleKind, ModuleResolutionKind, ScriptTarget, typescriptDefaults } from 'monaco-editor/languages/features/typescript/register';
import EditorWorker from 'monaco-editor/editor/editor.worker?worker';
import JsonWorker from 'monaco-editor/language/json/json.worker?worker';
import TypeScriptWorker from 'monaco-editor/language/typescript/ts.worker?worker';
import { JSON_SCHEMA_META, META_SCHEMA_URI } from './jsonSchemaMeta';
import { OCTOPUS_DTS, OCTOPUS_DTS_PATH } from './octopusTypes';
import { installMonacoEnvironment, modelAddress, resolveTheme, severityOf, studioThemeData, studioThemeName, tidyProblems } from './monacoSetup';
import type { ModelRole, Problem } from './monacoSetup';

installMonacoEnvironment(globalThis, {
  editor: EditorWorker,
  json: JsonWorker,
  typescript: TypeScriptWorker,
});

// TypeScript as the runner takes it: an ES module, no DOM, no Node typings. The platform erases the
// types and does not check them, so what the editor reports is advice; anything that is only
// "cannot find module" is not worth underlining (the sandbox has Node's own modules).
typescriptDefaults.setCompilerOptions({
  target: ScriptTarget.ESNext,
  module: ModuleKind.ESNext,
  moduleResolution: ModuleResolutionKind.NodeJs,
  allowNonTsExtensions: true,
  strict: true,
  noEmit: true,
  lib: ['esnext'],
});
typescriptDefaults.setDiagnosticsOptions({ noSemanticValidation: false, noSyntaxValidation: false, diagnosticCodesToIgnore: [2307] });
typescriptDefaults.addExtraLib(OCTOPUS_DTS, OCTOPUS_DTS_PATH);

/** Schemas the JSON service validates models against: the meta-schema for schema editors, the author's schema for the input. */
const jsonAssociations = new Map<string, { uri: string; schema: object }>();

function applyJsonSchemas(): void {
  const meta = [...jsonAssociations.entries()].filter(([, entry]) => entry.uri === META_SCHEMA_URI).map(([model]) => model);
  const schemas: { uri: string; fileMatch: string[]; schema: object }[] = [];
  if (meta.length > 0) schemas.push({ uri: META_SCHEMA_URI, fileMatch: meta, schema: JSON_SCHEMA_META });
  for (const [model, entry] of jsonAssociations) {
    if (entry.uri !== META_SCHEMA_URI) schemas.push({ uri: entry.uri, fileMatch: [model], schema: entry.schema });
  }
  jsonDefaults.setDiagnosticsOptions({
    validate: true,
    allowComments: false,
    trailingCommas: 'error',
    // Nothing is fetched: a reference outside the document is a problem, not a request.
    enableSchemaRequest: false,
    schemas,
  });
}

export type JsonSchemaFor = { kind: 'meta' } | { kind: 'schema'; schema: object } | undefined;

export interface EditorInit {
  role: ModelRole;
  /** The language of the model (`typescript`, `python`, `json`, `plaintext`...). */
  language: string;
  value: string;
  readOnly: boolean;
  ariaLabel: string;
  onChange: (value: string) => void;
  onProblems: (problems: Problem[]) => void;
  jsonSchema: JsonSchemaFor;
}

export interface EditorHandle {
  /** Replaces the text from outside (keeps undo working). */
  setValue: (value: string) => void;
  setReadOnly: (readOnly: boolean) => void;
  setJsonSchema: (schema: JsonSchemaFor) => void;
  setTabMovesFocus: (moves: boolean) => void;
  focus: () => void;
  reveal: (line: number, column: number) => void;
  dispose: () => void;
}

const FONT_FALLBACK = 'ui-monospace, SFMono-Regular, Menlo, Consolas, monospace';

function pageToken(name: string): string {
  return globalThis.getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

function currentTheme() {
  const base = resolveTheme(
    document.documentElement.getAttribute('data-theme'),
    globalThis.matchMedia('(prefers-color-scheme: dark)').matches,
    globalThis.matchMedia('(forced-colors: active)').matches,
  );
  const name = studioThemeName(base);
  if (base === 'vs' || base === 'vs-dark') monaco.editor.defineTheme(name, studioThemeData(base, pageToken('--fg-muted')));
  return name;
}

let themeWatch: { count: number; stop: () => void } | undefined;

/** Keeps Monaco's (global) theme in step with the page's: its own switch and the system's. */
function watchTheme(): () => void {
  if (themeWatch === undefined) {
    const apply = () => monaco.editor.setTheme(currentTheme());
    apply();
    const colorScheme = globalThis.matchMedia('(prefers-color-scheme: dark)');
    const forcedColors = globalThis.matchMedia('(forced-colors: active)');
    const observer = new MutationObserver(apply);
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
    colorScheme.addEventListener('change', apply);
    forcedColors.addEventListener('change', apply);
    themeWatch = {
      count: 0,
      stop: () => {
        observer.disconnect();
        colorScheme.removeEventListener('change', apply);
        forcedColors.removeEventListener('change', apply);
      },
    };
  }
  const watch = themeWatch;
  watch.count += 1;
  return () => {
    watch.count -= 1;
    if (watch.count === 0) {
      watch.stop();
      themeWatch = undefined;
    }
  };
}

export function createEditor(host: HTMLElement, init: EditorInit): EditorHandle {
  const address = modelAddress(init.role, init.language);
  const uri = monaco.Uri.parse(address);
  // A model of the same address left from a previous mount would keep its text.
  monaco.editor.getModel(uri)?.dispose();
  const model = monaco.editor.createModel(init.value, init.language, uri);
  model.updateOptions({ tabSize: init.language === 'python' ? 4 : 2, insertSpaces: true });
  const editor = monaco.editor.create(host, {
    model,
    ariaLabel: init.ariaLabel,
    readOnly: init.readOnly,
    domReadOnly: init.readOnly,
    // Screen-reader mode follows what the browser reports; a person can switch it in the editor's own help.
    accessibilitySupport: 'auto',
    automaticLayout: true,
    minimap: { enabled: false },
    scrollBeyondLastLine: false,
    fontFamily: pageToken('--font-mono') || FONT_FALLBACK,
    fontSize: 14,
    lineNumbers: 'on',
    renderWhitespace: 'selection',
    wordWrap: 'off',
    padding: { top: 8, bottom: 8 },
    // Text in a script is not a link to follow, and the page opens nothing outside itself.
    links: false,
    // Hidden and look-alike characters are shown, not trusted.
    unicodeHighlight: { ambiguousCharacters: true, invisibleCharacters: true, nonBasicASCII: false },
    renderControlCharacters: true,
    unusualLineTerminators: 'off',
    // Very long lines are cut when drawn (the text itself is kept whole).
    stopRenderingLineAfter: 10_000,
    fixedOverflowWidgets: true,
    theme: currentTheme(),
  });
  const stopTheme = watchTheme();

  let applying = false;
  const content = model.onDidChangeContent(() => {
    if (!applying) init.onChange(model.getValue());
  });
  const publishProblems = () => {
    init.onProblems(
      tidyProblems(
        monaco.editor.getModelMarkers({ resource: uri }).map((marker) => ({
          severity: severityOf(marker.severity),
          line: marker.startLineNumber,
          column: marker.startColumn,
          message: marker.message,
          ...(marker.source ? { source: marker.source } : {}),
        })),
      ),
    );
  };
  const markers = monaco.editor.onDidChangeMarkers((changed) => {
    if (changed.some((resource) => resource.toString() === uri.toString())) publishProblems();
  });

  const handle: EditorHandle = {
    setValue: (value) => {
      if (model.getValue() === value) return;
      applying = true;
      try {
        model.pushEditOperations([], [{ range: model.getFullModelRange(), text: value }], () => null);
      } finally {
        applying = false;
      }
    },
    setReadOnly: (readOnly) => editor.updateOptions({ readOnly, domReadOnly: readOnly }),
    setJsonSchema: (schema) => {
      const key = uri.toString();
      if (schema === undefined) jsonAssociations.delete(key);
      else if (schema.kind === 'meta') jsonAssociations.set(key, { uri: META_SCHEMA_URI, schema: JSON_SCHEMA_META });
      else jsonAssociations.set(key, { uri: `inmemory://sagat/schemas/${init.role}`, schema: schema.schema });
      applyJsonSchemas();
    },
    setTabMovesFocus: (moves) => editor.updateOptions({ tabFocusMode: moves }),
    focus: () => editor.focus(),
    reveal: (line, column) => {
      editor.setPosition({ lineNumber: line, column });
      editor.revealPositionInCenter({ lineNumber: line, column });
      editor.focus();
    },
    dispose: () => {
      handle.setJsonSchema(undefined);
      markers.dispose();
      content.dispose();
      stopTheme();
      editor.dispose();
      model.dispose();
    },
  };
  if (init.language === 'json') handle.setJsonSchema(init.jsonSchema);
  publishProblems();
  return handle;
}
