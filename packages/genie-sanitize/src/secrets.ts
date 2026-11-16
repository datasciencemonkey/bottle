/**
 * G-36: the secret patterns of section 4.6 rule 7. A question that matches one of them
 * does not go; the bridge reports SECRET_IN_QUESTION.
 */
export const SECRET_PATTERNS: readonly { name: string; re: RegExp }[] = [
  { name: 'jwt', re: /eyJ[\w-]+\./ },
  { name: 'databricks_pat', re: /dapi[0-9a-f]{32}/ },
  { name: 'databricks_oauth', re: /dose[0-9a-f]+/ },
  { name: 'aws_access_key', re: /AKIA[0-9A-Z]{16}/ },
  { name: 'github_pat', re: /ghp_[A-Za-z0-9]+/ },
  { name: 'pem_block', re: /-----BEGIN/ },
  { name: 'password_assignment', re: /password=/ },
];

/** Returns the name of the first matching pattern (never the matched text), or undefined. */
export function findSecret(text: string): string | undefined {
  return SECRET_PATTERNS.find((p) => p.re.test(text))?.name;
}
