import { signRelease } from './sign-release.mjs';

try { signRelease('0.2.1'); }
catch (error) { console.error(error.message); process.exitCode = 1; }
