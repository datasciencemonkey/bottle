/** The preamble shape of research/final-requirements.md section 4.7 (built by the ContextAssembler). */
export interface ContextPreamble {
  header: 'Context (from my notes, may be wrong):';
  items: {
    id: string;
    text: string;
    source: 'you' | 'confirmed';
    ts: number;
    truncated: boolean;
  }[];
  chars: number;
}

/** The subset of the section 4.7 `BackendErrorCode` union that this package returns. */
export type SanitizeErrorCode = 'SECRET_IN_QUESTION' | 'CLIENT_BUG';

export interface Blocked<R extends string> {
  ok: false;
  code: SanitizeErrorCode;
  reason: R;
  userText: string;
}
