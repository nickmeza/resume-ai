// Copies the browser files of xterm.js into ui/vendor (the app serves ui/ as-is, without a bundler).
const fs = require('fs');
const path = require('path');

const root = path.join(__dirname, '..');
const out = path.join(root, 'ui', 'vendor');
const files = [
  ['@xterm/xterm/lib/xterm.js', 'xterm.js'],
  ['@xterm/xterm/css/xterm.css', 'xterm.css'],
  ['@xterm/addon-fit/lib/addon-fit.js', 'addon-fit.js'],
];

fs.mkdirSync(out, { recursive: true });
for (const [from, to] of files) {
  fs.copyFileSync(path.join(root, 'node_modules', from), path.join(out, to));
}
console.log(`vendor: copied ${files.length} files to ui/vendor`);
