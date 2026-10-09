import { signRelease } from './sign-release.mjs';

try { signRelease('0.2.2'); }
catch (error) { console.error(error.message); process.exitCode = 1; }
