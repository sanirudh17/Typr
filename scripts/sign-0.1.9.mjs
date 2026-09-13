import { execSync } from 'child_process';
import path from 'path';

const env = {
  ...process.env,
  TAURI_SIGNING_PRIVATE_KEY: "dW50cnVzdGVkIGNvbW1lbnQ6IHJzaWduIGVuY3J5cHRlZCBzZWNyZXQga2V5ClJXUlRZMEl5ME9mR29aSzRoZzFMRHZxTzlqWEkyeEVHaW1XaEdEQzRiNTc1RUxUdjdFd0FBQkFBQUFBQUFBQUFBQUlBQUFBQVBFMnZvN243K2pLT1hUS2Z2M21KTnFUaVdyOWZnMnN1UEpRbEJYRzVNcnptRXIrRzI1dlNocksrOEJiU1Z5WWZ6TGJwWEltUTBjZEh6ejhCSU9FVnlzUFdRR3c2U3V1OE5KcStXMndVTHE2bkNFN29TeE1mcXk3RzcxeDJwWUYzT2djVFQ1Ry9Fd1U9Cg==",
  TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ""
};

const target = path.resolve('src-tauri/target/release/bundle/nsis/Typr_0.1.9_x64-setup.exe');
console.log(`Signing ${target}...`);
const output = execSync(`npx tauri signer sign "${target}"`, { env, stdio: 'pipe' });
console.log(output.toString());
