/**
 * Settings, "Dictation words": the user's own list of words the local speech engine mishears.
 * Each line is "heard → write". The list is saved in local settings (localStorage, as the
 * sidebar choice is) and the brain reads it on each transcript (`shared/dictation-vocab.ts`).
 * A built-in list of Databricks terms always applies; this list adds to it and wins over it.
 */
import { useId, useState } from 'react';
import {
  parseUserWords,
  readUserWordsText,
  writeUserWordsText,
} from '../../shared/dictation-vocab';
import { Textarea } from '../ui/textarea';
import { SettingsGroup } from './SettingsGroup';

export const DICTATION_WORDS_PLACEHOLDER = 'unity cat log → Unity Catalog\nsathish → Sathish';

export function DictationWords() {
  const [text, setText] = useState(() => readUserWordsText());
  const areaId = useId();
  const helpId = useId();
  const { rules, invalid } = parseUserWords(text);

  const change = (next: string) => {
    setText(next);
    writeUserWordsText(next);
  };

  return (
    <section
      data-view="dictation-words"
      aria-labelledby="dictation-words-title"
      className="hub-section"
    >
      <h2 id="dictation-words-title">Dictation words</h2>
      <SettingsGroup className="mt-[var(--mog-space-3)]">
        <div className="flex flex-col gap-[var(--mog-space-2)] px-[var(--mog-space-4)] py-[var(--mog-space-3)]">
          <label htmlFor={areaId} className="flex flex-col gap-[var(--mog-space-1)]">
            <span className="text-[var(--mog-text-md)]">
              Fix words the speech engine gets wrong
            </span>
            <span id={helpId} className="text-[var(--mog-text-xs)] text-muted-foreground">
              One per line: what it heard → what to write. Bottle already fixes common Databricks
              terms such as Gateway, Unity Catalog and Lakebase.
            </span>
          </label>
          <Textarea
            id={areaId}
            aria-describedby={helpId}
            name="dictationWords"
            rows={4}
            spellCheck={false}
            autoCapitalize="off"
            autoCorrect="off"
            className="min-h-24 resize-y font-mono"
            placeholder={DICTATION_WORDS_PLACEHOLDER}
            value={text}
            onChange={(e) => change(e.target.value)}
          />
          <p
            data-dictation-words-status
            aria-live="polite"
            className={
              invalid > 0
                ? 'text-[var(--mog-text-xs)] text-destructive'
                : 'text-[var(--mog-text-xs)] text-muted-foreground'
            }
          >
            {invalid > 0
              ? `${invalid} ${invalid === 1 ? 'line is' : 'lines are'} not used. Write each as: heard → write.`
              : rules.length === 0
                ? 'No words of your own yet.'
                : `${rules.length} ${rules.length === 1 ? 'word' : 'words'} saved.`}
          </p>
        </div>
      </SettingsGroup>
    </section>
  );
}
