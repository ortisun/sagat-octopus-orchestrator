import { useAnnounce } from '../../ui/Announcer';

/**
 * Copies `text` to the clipboard and says so. Only ids and digests go through here, never a
 * payload or a secret. Where the browser will not allow it, it says that too: the text is on the
 * page to be selected.
 */
export function CopyButton({ text, what }: { text: string; what: string }) {
  const announce = useAnnounce();
  const copy = () => {
    const clipboard = globalThis.navigator?.clipboard;
    if (clipboard === undefined) {
      announce(`Could not copy ${what}: select it and copy it by hand.`);
      return;
    }
    clipboard.writeText(text).then(
      () => announce(`Copied ${what}.`),
      () => announce(`Could not copy ${what}: select it and copy it by hand.`),
    );
  };
  return (
    <button type="button" className="btn small" onClick={copy} aria-label={`Copy ${what}`}>
      Copy
    </button>
  );
}
