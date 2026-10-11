import { useState } from 'react';
import { useAnnounce } from '../../ui/Announcer';
import { Button } from '../../ui/Button';

/**
 * Copies `text` (never a secret: a digest, an id) to the clipboard and says so. Where the browser
 * will not allow it, it says that too, and the text is on the page to be selected.
 */
export function CopyButton({ text, label, what }: { text: string; label: string; what: string }) {
  const announce = useAnnounce();
  const [failed, setFailed] = useState(false);
  const copy = () => {
    const clipboard = globalThis.navigator?.clipboard;
    if (clipboard === undefined) {
      setFailed(true);
      announce(`Could not copy ${what}: select it and copy it by hand.`);
      return;
    }
    clipboard.writeText(text).then(
      () => {
        setFailed(false);
        announce(`Copied ${what}.`);
      },
      () => {
        setFailed(true);
        announce(`Could not copy ${what}: select it and copy it by hand.`);
      },
    );
  };
  return (
    <>
      <Button onClick={copy}>{label}</Button>
      {failed ? <span className="hint"> The browser did not allow copying; select the text and copy it.</span> : null}
    </>
  );
}
