import type { ComponentType } from 'react';
import type { MonacoEditorProps } from './MonacoEditor';

/** What loading the editor gives: the component that wraps Monaco. */
export interface EditorModule {
  MonacoEditor: ComponentType<MonacoEditorProps>;
}

/**
 * Loads Monaco and its wrapper, once, when an editor is first shown. This is the only place that
 * names the chunk, which keeps Monaco out of the first download, and the place a test replaces
 * (`vi.mock('./editorModule')`): jsdom cannot run Monaco, so the tests either make this fail, to
 * exercise the plain text box, or return a stand-in, to exercise the wiring.
 */
export function loadEditor(): Promise<EditorModule> {
  return import('./MonacoEditor');
}
