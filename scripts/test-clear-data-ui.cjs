// Browser fixtures only: no Tauri process, actual history, or credentials are used.
// Usage: node scripts/test-clear-data-ui.cjs <playwright module> <browser executable> <preview URL> <output directory>
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');

async function assertReadable(page, selector) {
  const ratio = await page.locator(selector).evaluate(el => {
    const canvas = document.createElement('canvas');
    canvas.width = canvas.height = 1;
    const context = canvas.getContext('2d');
    const luminance = color => {
      context.clearRect(0, 0, 1, 1);
      context.fillStyle = color;
      context.fillRect(0, 0, 1, 1);
      const rgb = [...context.getImageData(0, 0, 1, 1).data].slice(0, 3)
        .map(value => value / 255).map(value => value <= 0.04045
          ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4);
      return rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722;
    };
    let background = el;
    while (getComputedStyle(background).backgroundColor === 'rgba(0, 0, 0, 0)') {
      background = background.parentElement;
    }
    const a = luminance(getComputedStyle(el).color);
    const b = luminance(getComputedStyle(background).backgroundColor);
    return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
  });
  assert.ok(ratio >= 4.5, `${selector} contrast ${ratio.toFixed(2)}:1 is below 4.5:1`);
}

async function main() {
  const [modulePath, executablePath, baseURL, outputDir] = process.argv.slice(2);
  if (!outputDir) throw new Error('Expected Playwright module, browser executable, preview URL, and output directory');
  const { chromium } = require(modulePath);
  await fs.mkdir(outputDir, { recursive: true });
  const browser = await chromium.launch({ executablePath, headless: true });
  try {
    for (const theme of ['dark', 'light']) {
      const page = await browser.newPage({ viewport: { width: 1160, height: 720 } });
      const errors = [];
      page.on('pageerror', error => errors.push(error.message));
      await page.addInitScript(({ theme }) => {
        const callbacks = new Map(), listeners = new Map();
        let id = 0;
        const settings = {
          microphone: 'Fixture microphone', engine: 'nemotron', whisperModel: 'small.en', groqApiKey: '',
          recordingMode: 'toggle', hotkey: 'Ctrl+Shift+Space', cloudModel: 'whisper-large-v3',
          parakeetModel: 'v3', nemotronModel: 'v3_5', aiEnabled: false, aiModel: 'qwen/qwen3.8-27b',
          aiProfile: 'cleanup', aiPromptFormat: 'natural', aiTone: 'default', aiFormat: 'default',
          aiCustomInstructions: '', backgroundMode: true, autostart: false, theme,
          hotkeySecondary: '', secondaryProfile: 'auto', autoContextOverride: 'auto', appRules: [],
          dismissedUpdateVersion: '',
        };
        window.reviewFixture = {
          history: { items: [{ id: 'fixture', timestamp: 1, text: 'Example transcription.', duration_secs: 1, word_count: 2 }] },
          clearCalls: 0, deleteCalls: 0, mode: 'success', finish: null,
          emit(event, payload) {
            for (const handler of listeners.get(event) || []) callbacks.get(handler)?.({ event, payload });
          },
        };
        window.__TYPR_BOOT__ = { theme, light: theme === 'light' };
        window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
        window.__TAURI_INTERNALS__ = {
          metadata: { currentWindow: { label: 'main' } },
          transformCallback(fn) { callbacks.set(++id, fn); return id; },
          unregisterCallback(key) { callbacks.delete(key); },
          async invoke(command, args) {
            if (command === 'plugin:event|listen') {
              listeners.set(args.event, [...(listeners.get(args.event) || []), args.handler]); return ++id;
            }
            if (command === 'get_settings') return structuredClone(settings);
            if (command === 'get_history') return structuredClone(window.reviewFixture.history);
            if (command === 'get_dictionary') return { vocabulary_hints: [], replacements: [] };
            if (command === 'list_microphones') return [{ name: settings.microphone, is_default: true }];
            if (command === 'get_app_rules' || command === 'list_running_apps') return [];
            if (command === 'get_recording_state') return 'Ready';
            if (command === 'plugin:app|version') return '0.2.2';
            if (command === 'plugin:window|theme') return theme;
            if (command === 'clear_local_data') {
              const fixture = window.reviewFixture;
              fixture.clearCalls++;
              if (fixture.mode === 'error') throw new Error('Fixture storage is unavailable. Try again.');
              if (fixture.mode === 'pending') await new Promise(resolve => { fixture.finish = resolve; });
              const count = fixture.history.items.length;
              fixture.history = { items: [] };
              fixture.emit('history-updated');
              return { history_items: count, files_removed: 3, bytes_freed: 1000, warnings: [] };
            }
            if (command === 'delete_transcription') window.reviewFixture.deleteCalls++;
            return null;
          },
        };
      }, { theme });
      await page.goto(baseURL);
      await page.locator('.nav-item[data-section="general"]').click();
      await page.locator('#clear-data-btn').click();
      assert.equal(await page.evaluate(() => document.activeElement.id), 'clear-data-cancel');
      await page.keyboard.press('Shift+Tab');
      assert.equal(await page.evaluate(() => document.activeElement.id), 'clear-data-confirm');
      await page.keyboard.press('Tab');
      assert.equal(await page.evaluate(() => document.activeElement.id), 'clear-data-cancel');
      for (const selector of ['#clear-data-description', '#clear-data-kept', '#clear-data-cancel', '#clear-data-confirm', '#clear-data-error']) {
        await assertReadable(page, selector);
      }
      await page.locator('#clear-data-confirm').hover();
      await page.waitForTimeout(250);
      await assertReadable(page, '#clear-data-confirm');
      await page.mouse.move(0, 0);
      await page.waitForTimeout(250);
      await page.evaluate(() => document.fonts.ready);
      await page.screenshot({ path: path.join(outputDir, `confirmation-${theme}.png`) });
      const colors = await page.locator('#clear-data-dialog').evaluate(el => {
        const style = getComputedStyle(el);
        return { background: style.backgroundColor, color: style.color, font: style.fontFamily };
      });
      assert.equal(colors.background, theme === 'light' ? 'rgb(255, 255, 255)' : 'rgb(19, 19, 22)');
      assert.match(colors.font, /IBM Plex Sans/);
      await page.setViewportSize({ width: 700, height: 500 });
      await page.screenshot({ path: path.join(outputDir, `confirmation-${theme}-small.png`) });
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
      await page.locator('#clear-data-cancel').click();
      assert.equal(await page.evaluate(() => window.reviewFixture.clearCalls), 0);
      assert.equal(await page.evaluate(() => document.activeElement.id), 'clear-data-btn');
      await page.locator('#clear-data-btn').click();
      await page.keyboard.press('Escape');
      assert.equal(await page.locator('#clear-data-dialog').evaluate(el => el.open), false);
      // Confirming must update history; an old Undo cannot restore it.
      await page.locator('.nav-item[data-section="home"]').click();
      await page.locator('.feed-item-del-btn').click();
      await page.locator('.nav-item[data-section="general"]').click();
      await page.locator('#clear-data-btn').click();
      await page.evaluate(() => { window.reviewFixture.mode = 'pending'; });
      await page.locator('#clear-data-confirm').click();
      await page.waitForFunction(() => window.reviewFixture.finish !== null);
      assert.equal(await page.locator('#clear-data-cancel').isDisabled(), true);
      await page.keyboard.press('Escape');
      assert.equal(await page.locator('#clear-data-dialog').evaluate(el => el.open), true);
      await page.evaluate(() => window.reviewFixture.finish());
      await page.waitForFunction(() => !document.getElementById('clear-data-dialog').open);
      assert.equal(await page.locator('.undo-toast-btn').count(), 0);
      assert.equal(await page.locator('#stat-count').innerText(), '0');
      await page.evaluate(() => { window.reviewFixture.mode = 'error'; });
      await page.locator('#clear-data-btn').click();
      await page.locator('#clear-data-confirm').click();
      await page.waitForFunction(() => !document.getElementById('clear-data-error').classList.contains('hidden'));
      assert.match(await page.locator('#clear-data-error').innerText(), /Could not clear data/);
      await page.locator('#clear-data-cancel').click();
      await page.evaluate(() => window.reviewFixture.emit('recording-state', 'Recording'));
      assert.equal(await page.locator('#clear-data-btn').isDisabled(), true);
      await page.evaluate(() => window.reviewFixture.emit('recording-state', 'Transcribing'));
      assert.equal(await page.locator('#clear-data-btn').isDisabled(), true);
      await page.evaluate(() => window.reviewFixture.emit('recording-state', 'Ready'));
      assert.equal(await page.locator('#clear-data-btn').isDisabled(), false);
      await page.waitForTimeout(5100); // Retired delete timer must never call the backend.
      assert.equal(await page.evaluate(() => window.reviewFixture.deleteCalls), 0);
      assert.deepEqual(errors, []);
      console.log(`PASS ${theme}: theme, minimum window, focus, cancel, pending, clear, history/undo, error, busy`);
      await page.close();
    }
  } finally { await browser.close(); }
}
main().catch(error => { console.error(error); process.exitCode = 1; });
