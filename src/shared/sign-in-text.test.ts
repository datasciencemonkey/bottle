import { describe, expect, it } from 'vitest';
import { signInCommand } from './sign-in-text';

describe('signInCommand', () => {
  it('leaves a plain name bare and quotes any other name for the shell', () => {
    expect(signInCommand('work-1.a_b')).toBe('databricks auth login --profile work-1.a_b');
    expect(signInCommand('my work')).toBe("databricks auth login --profile 'my work'");
    expect(signInCommand("o'x")).toBe("databricks auth login --profile 'o'\\''x'");
  });
});
