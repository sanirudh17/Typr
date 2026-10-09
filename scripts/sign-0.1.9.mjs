import { signRelease } from './sign-release.mjs';

try { signRelease('0.1.9'); }
catch (error) { console.error(error.message); process.exitCode = 1; }
