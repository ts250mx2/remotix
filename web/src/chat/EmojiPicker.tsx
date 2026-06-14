import { useEffect, useRef, useState } from 'react';

const EMOJIS = [
  '😀', '😁', '😂', '🤣', '😊', '😉', '😍', '😎', '🤔', '😴',
  '😅', '😇', '🙂', '🙃', '😌', '😢', '😭', '😡', '😱', '🤯',
  '👍', '👎', '👌', '🙌', '👏', '🙏', '💪', '🤝', '👋', '✌️',
  '🔥', '✅', '❌', '⚠️', '❗', '❓', '💡', '⭐', '🎉', '✨',
  '❤️', '💔', '💯', '👀', '🚀', '🛠️', '🖥️', '💻', '📎', '📌',
  '⏰', '✔️', '➡️', '🔄', '🔒', '🔓', '📞', '📷', '🟢', '🔴',
];

export function EmojiPicker({ onPick }: { onPick: (e: string) => void }) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener('mousedown', onDoc);
    return () => document.removeEventListener('mousedown', onDoc);
  }, [open]);

  return (
    <div className="emoji" ref={ref}>
      <button type="button" className="ghost" title="Emojis" onClick={() => setOpen((v) => !v)}>😀</button>
      {open && (
        <div className="emoji-pop">
          {EMOJIS.map((e) => (
            <button type="button" key={e} className="emoji-btn" onClick={() => { onPick(e); setOpen(false); }}>
              {e}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
