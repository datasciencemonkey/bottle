/**
 * The UC Page domain picker (thin, PG-20). It renders the list or the paste field that the brain
 * sends and returns the pick or the raw pasted text with `view_emit`. The brain parses the text
 * and saves the home domain (PG-18) or the one-draft change (PG-19).
 */
import { useState } from 'react';
import type { MogId } from '../shared/interfaces';
import { emitView, type DomainPickerView } from './view-ipc';

export function DomainPicker({ mogId, picker }: { mogId: MogId; picker: DomainPickerView }) {
  const [text, setText] = useState('');
  const label = picker.scope === 'home' ? 'Home domain' : 'Domain for this draft';
  const where =
    picker.scope === 'draft' ? { scope: 'draft', reviewId: picker.reviewId } : { scope: 'home' };
  const send = (payload: Record<string, unknown>) =>
    void emitView(mogId, 'click', { ...payload, ...where });

  return (
    <div data-domain-picker={picker.scope}>
      <span>{label}</span>
      {picker.options ? (
        <select
          aria-label={label}
          value={picker.options.some((o) => o.id === picker.current?.id) ? picker.current?.id : ''}
          onChange={(e) => send({ target: 'domain', domainId: e.target.value })}
        >
          {!picker.options.some((o) => o.id === picker.current?.id) && (
            <option value="">Pick a domain</option>
          )}
          {picker.options.map((o) => (
            <option key={o.id} value={o.id}>
              {o.name}
            </option>
          ))}
        </select>
      ) : (
        <>
          {picker.current && (
            <span data-domain-current>{picker.current.name ?? picker.current.id}</span>
          )}
          {picker.paste !== undefined && <p>{picker.paste}</p>}
          <input
            type="text"
            aria-label="Domain ID or Catalog Explorer URL"
            value={text}
            onChange={(e) => setText(e.target.value)}
          />
          <button type="button" onClick={() => send({ target: 'domain_paste', text })}>
            Use this domain
          </button>
        </>
      )}
      {picker.error !== undefined && <p role="alert">{picker.error}</p>}
    </div>
  );
}
