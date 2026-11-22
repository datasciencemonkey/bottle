/**
 * The add and edit form of the hub Memory area (memory slice 1, S3 and S4; ADR 0113). One body
 * field (focused), an optional title and a kind. ⌘↵ (or Ctrl+↵) saves, Esc cancels. The form
 * keeps its text when the brain refuses the save, so a secret never costs the user their words.
 * Thin view (ARC-2): it only collects the fields. The brain checks them (secret check, caps).
 */
import { useEffect, useId, useRef, useState, type KeyboardEvent } from 'react';
import {
  BODY_MAX,
  EDITABLE_KINDS,
  KIND_LABEL,
  TITLE_MAX,
  type EditableKind,
  type MemoryKindName,
} from '../../shared/hub-memory';
import { Button } from '../ui/button';
import { Input } from '../ui/input';
import { Textarea } from '../ui/textarea';

/** The first line of `body`, at most {@link TITLE_MAX} characters, cut at a word. */
export function deriveTitle(body: string): string {
  const line =
    body
      .split('\n')
      .find((l) => l.trim() !== '')
      ?.trim() ?? '';
  if (line.length <= TITLE_MAX) return line;
  const cut = line.slice(0, TITLE_MAX);
  // The cut falls inside a word when the next character is not a space: drop that part word.
  const at = line[TITLE_MAX] === ' ' ? -1 : cut.lastIndexOf(' ');
  return (at > 0 ? cut.slice(0, at) : cut).trim();
}

export interface MemoryFormValues {
  kind: MemoryKindName;
  title: string;
  body: string;
}

export interface MemoryFormProps {
  mode: 'add' | 'edit';
  initial: MemoryFormValues;
  busy: boolean;
  /** The message under the form, for example the secret message. */
  error?: string | undefined;
  onSubmit: (v: MemoryFormValues) => void;
  onCancel: () => void;
}

const isEditable = (k: MemoryKindName): k is EditableKind =>
  (EDITABLE_KINDS as readonly string[]).includes(k);

export function MemoryForm({ mode, initial, busy, error, onSubmit, onCancel }: MemoryFormProps) {
  const [body, setBody] = useState(initial.body);
  const [title, setTitle] = useState(initial.title);
  // In the add form the title follows the first line of the body until the user types in it.
  const [titleTouched, setTitleTouched] = useState(mode === 'edit');
  const [kind, setKind] = useState<MemoryKindName>(initial.kind);
  const bodyRef = useRef<HTMLTextAreaElement>(null);
  const id = useId();

  useEffect(() => {
    const el = bodyRef.current;
    el?.focus();
    // In the edit form the cursor starts at the end of the text.
    if (el && mode === 'edit') el.setSelectionRange(el.value.length, el.value.length);
  }, [mode]);

  const shownTitle = titleTouched ? title : deriveTitle(body);
  const canSave = !busy && body.trim() !== '';
  const submit = () => {
    if (!canSave) return;
    onSubmit({ kind, title: shownTitle.trim().slice(0, TITLE_MAX), body });
  };
  const onKeyDown = (e: KeyboardEvent<HTMLFormElement>) => {
    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      submit();
    } else if (e.key === 'Escape' && !e.nativeEvent.isComposing) {
      e.preventDefault();
      e.stopPropagation();
      onCancel();
    }
  };

  return (
    <form
      className="mem-form"
      aria-label={mode === 'add' ? 'New memory' : 'Edit memory'}
      data-memory-form={mode}
      onKeyDown={onKeyDown}
      onSubmit={(e) => {
        e.preventDefault();
        submit();
      }}
    >
      <label className="mem-form-label" htmlFor={`${id}-body`}>
        What should Bottle remember?
      </label>
      <Textarea
        id={`${id}-body`}
        ref={bodyRef}
        className="mem-form-body"
        value={body}
        maxLength={BODY_MAX}
        rows={7}
        placeholder="For example: the sales mart refreshes at six."
        data-field="body"
        onChange={(e) => setBody(e.target.value)}
      />
      <div className="mem-form-row">
        <div className="mem-form-col">
          <label className="mem-form-label" htmlFor={`${id}-title`}>
            Title <span className="mem-form-hint">optional</span>
          </label>
          <Input
            id={`${id}-title`}
            value={shownTitle}
            maxLength={TITLE_MAX}
            placeholder="First line of the text"
            data-field="title"
            onChange={(e) => {
              setTitle(e.target.value);
              setTitleTouched(true);
            }}
          />
        </div>
        <div className="mem-form-col mem-form-col--kind">
          <label className="mem-form-label" htmlFor={`${id}-kind`}>
            Kind
          </label>
          {isEditable(kind) ? (
            <select
              id={`${id}-kind`}
              className="mem-select"
              value={kind}
              data-field="kind"
              onChange={(e) => setKind(e.target.value as EditableKind)}
            >
              {EDITABLE_KINDS.map((k) => (
                <option key={k} value={k}>
                  {KIND_LABEL[k]}
                </option>
              ))}
            </select>
          ) : (
            <span id={`${id}-kind`} className="mem-form-fixed" data-field="kind-fixed">
              {KIND_LABEL[kind]}
            </span>
          )}
        </div>
      </div>
      {error && (
        <p className="mem-form-error" role="alert" data-memory-form-error>
          {error}
        </p>
      )}
      <div className="mem-form-actions">
        <Button type="submit" size="sm" disabled={!canSave} data-action="form-save">
          {mode === 'add' ? 'Save memory' : 'Save changes'}
          <kbd className="mem-kbd" aria-hidden="true">
            ⌘↵
          </kbd>
        </Button>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          onClick={onCancel}
          data-action="form-cancel"
        >
          Cancel
          <kbd className="mem-kbd" aria-hidden="true">
            Esc
          </kbd>
        </Button>
      </div>
    </form>
  );
}
