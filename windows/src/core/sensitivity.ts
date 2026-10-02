// How dangerous is the thing Nouve is asking permission for?
//
// The approval card is the one moment we know exactly what is about to happen —
// `State.pendingApproval.command` carries the file path or the shell command
// straight from the hook (`approvalTarget()` in island/hooks.ts) or from the chat
// tool loop (claude.rs `approval_target`). That string is all we need: no extra
// backend data, no polling, just a read of what the human is being asked to
// authorise.
//
// The result drives the card wash: `attention` → amber, `danger` → red. Anything
// that isn't classified as dangerous is `attention`, because an approval that
// isn't risky is still an approval — it should never look like nothing.

export type RiskLevel = "attention" | "danger";

/**
 * Files whose contents are secrets by nature. Matching is deliberately narrow —
 * a false red every time someone writes a file with "key" in the name would make
 * the signal worthless, which is worse than missing an edge case.
 */
const DANGER_PATHS: RegExp[] = [
  // .env, .env.local, .env.production …
  /(^|[\\/\s'"`])\.env(\.[a-z0-9]+)?($|[\\/\s'"`:])/i,
  // Certificates and private keys.
  /\.(pem|key|pfx|p12|jks|keystore|kdbx|ppk|asc|gpg)\b/i,
  /\bid_(rsa|dsa|ecdsa|ed25519)\b/i,
  // SSH / cloud / GPG credential stores.
  /[\\/]\.ssh([\\/]|$)/i,
  /[\\/]\.aws([\\/]|$)/i,
  /[\\/]\.gnupg([\\/]|$)/i,
  /[\\/]\.kube([\\/]|$)/i,
  // Named credential files.
  /\bcredentials?(\.[a-z0-9]+)?\b/i,
  /\.npmrc\b|\.netrc\b|\.git-credentials\b|\.pgpass\b|\.htpasswd\b/i,
  /\bsecrets?\.(json|ya?ml|toml|env|ini)\b/i,
  // Crypto wallets and recovery phrases.
  /\b(wallet|keystore|mnemonic|seed[-_ ]?phrase)s?\b/i,
  /\bauthorized_keys\b|\bknown_hosts\b/i,
];

/**
 * Commands that destroy data, escape the sandbox, or change the machine's
 * security posture. Matched against the whole target string, so `Bash · rm -rf /`
 * and a bare `Remove-Item -Recurse -Force …` both land here.
 */
const DANGER_COMMANDS: RegExp[] = [
  // Recursive force delete (POSIX + PowerShell + cmd).
  /\brm\s+(-[a-z]*\s+)*-[a-z]*r[a-z]*f|\brm\s+(-[a-z]*\s+)*-[a-z]*f[a-z]*r/i,
  /\bremove-item\b(?=[\s\S]*(-recurse|-force))/i,
  /\bdel\s+\/[a-z]*[fsq]/i,
  /\b(rd|rmdir)\s+\/[a-z]*s/i,
  // Disk-level destruction.
  /\bformat\s+[a-z]:/i,
  /\bdiskpart\b|\bmkfs\b|\bdd\s+if=.*\bof=\/dev\//i,
  /\bvssadmin\s+delete\b|\bcipher\s+\/w\b/i,
  // Databases.
  /\bdrop\s+(table|database|schema)\b|\btruncate\s+table\b/i,
  // Pipe-to-shell: the classic remote code execution.
  /\b(curl|wget)\b[\s\S]*\|\s*(ba|z|k)?sh\b/i,
  /\b(iwr|invoke-webrequest)\b[\s\S]*\|\s*(iex|invoke-expression)\b/i,
  /\binvoke-expression\b|\biex\s*[(\s]/i,
  // Machine security posture.
  /\breg\s+delete\b|\bbcdedit\b|\btakeown\b|\bset-executionpolicy\b/i,
  /\bnet\s+user\b[\s\S]*\/add\b/i,
  /\bschtasks\b[\s\S]*\/create\b/i,
  /\bshutdown\b|\brestart-computer\b|\bstop-computer\b/i,
  // Fork bomb.
  /:\(\)\s*\{.*\};\s*:/,
];

/**
 * @param tool   The tool asking (Bash, Write, Edit, ohmypii …).
 * @param target The string the user is being asked to authorise: a command, a
 *   file path, or a URL, as shown on the card.
 */
export function classifyApproval(tool: string, target: string): RiskLevel {
  const haystack = `${tool} ${target}`;
  for (const re of DANGER_PATHS) if (re.test(haystack)) return "danger";
  for (const re of DANGER_COMMANDS) if (re.test(haystack)) return "danger";
  return "attention";
}
