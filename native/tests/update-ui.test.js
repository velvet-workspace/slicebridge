'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const frontend = path.join(__dirname, '..', 'frontend', 'dist');
const html = fs.readFileSync(path.join(frontend, 'index.html'), 'utf8');
const source = fs.readFileSync(path.join(frontend, 'app.js'), 'utf8');

function element(id) {
  return {
    id, textContent: '', title: '', className: '', dataset: {}, style: {}, value: '',
    hidden: false, disabled: false, checked: false, open: false,
    children: [], handlers: {},
    classList: { toggle() {}, add() {}, remove() {} },
    addEventListener(type, fn) { this.handlers[type] = fn; },
    setAttribute() {},
    removeAttribute(name) { if (name === 'data-status') delete this.dataset.status; },
    replaceChildren(...children) { this.children = children; },
    append(...children) { this.children.push(...children); },
    appendChild(child) { this.children.push(child); },
    querySelector() { return element('nested'); },
    showModal() { this.open = true; },
    close() { this.open = false; },
    getBoundingClientRect() { return {width: 650, height: 500}; }
  };
}

function setupUI() {
  for (const id of ['checkUpdatesBtn', 'updateNotice', 'updateStatus', 'openUpdateBtn']) {
    assert.match(html, new RegExp('id="' + id + '"'), 'update controls must remain in native UI');
  }
  const ids = [
    'fileList', 'queueCount', 'queuedFilesLabel', 'queueFilesShownCount',
    'resultList', 'reportCounts', 'convertBtn',
    'rustNativeDevControl', 'rustNativeOptIn', 'cancelRustConversionBtn',
    'convertCount', 'activityText', 'activityDetail', 'activityIcon',
    'openFolderBtn', 'settingsDialog', 'clearQueueBtn', 'addFilesBtn',
    'addFolderBtn', 'selectAllBtn', 'settingsBtn', 'myPrintersBtn',
    'emptyTargetNotice', 'resultsPanel',
    'clearResultsBtn', 'installedVersion', 'checkUpdatesBtn',
    'updateNotice', 'updateStatus', 'openUpdateBtn', 'dismissUpdateBtn',
    'heBtn', 'enBtn', 'settingsTabs', 'settingsNozzle', 'settingsSlicer',
    'settingsMaterial', 'settingsFilament', 'settingsStatus',
    'closeSettingsBtn', 'cancelSettingsBtn', 'saveSettingsBtn',
    'dropzone', 'targetList',
    'progressDescription', 'progressPercent', 'progressTrack', 'progressFill'
  ];
  const elements = Object.fromEntries(ids.map((id) => [id, element(id)]));
  // The queue list is a direct child of the actual source panel. The renderer
  // toggles its has-files class when files are added/removed.
  elements.fileList.parentElement = element('source-panel');
  const targets = ['u1', 'c5', 'cc2', 'qidi'];
  const rows = Object.fromEntries(targets.map((target) => {
    const checkbox = element(target + '-checkbox');
    checkbox.value = target;
    checkbox.checked = true;
    const summary = element(target + '-summary');
    const row = element(target + '-row');
    row.querySelector = (selector) => selector === 'input' ? checkbox : summary;
    return [target, {checkbox, summary, row, gear: element(target + '-gear')}];
  }));
  const sent = [];
  const app = {
    GetSettings: async () => ({
      version: 1, window: {width: 1100, height: 750},
      printers: Object.fromEntries(targets.map(target => [target, {
        nozzle: 'auto', slicer: 'auto', filaments: {}
      }]))
    }),
    Availability: async () => targets.map(target => ({
      target, name: target, available: true, slicer: 'installed'
    })),
    GetVersion: async () => ({
      version: '0.4.0-beta.3', commit: 'abc123', platform: 'windows-amd64'
    }),
    CheckForUpdates: async () => ({
      status: 'update_available', latestVersion: '0.4.0-beta.4',
      releaseTag: 'v0.4.0-beta.4', assetName: '3MF-Bridge-Windows-native.zip'
    }),
    OpenUpdatePage: async (kind, tag) => {sent.push({kind, tag});}
  };
  const docHandlers = {};
  const document = {
    getElementById(id) {
      assert.ok(elements[id], 'unexpected ID read by JS: ' + id);
      return elements[id];
    },
    querySelector(selector) {
      const target = selector.match(/data-(?:target-row|settings-target)="([^"]+)"/)?.[1];
      if (!target) return null;
      assert.ok(rows[target]);
      return selector.startsWith('[data-settings-target') ? rows[target].gear : rows[target].row;
    },
    querySelectorAll(selector) {
      if (selector.startsWith('#targetList input')) {
        return targets.map(target => rows[target].checkbox)
          .filter(x => x.checked && !x.disabled);
      }
      return [];
    },
    createElement: () => element('new'),
    createDocumentFragment: () => element('fragment'),
    documentElement: {lang: 'en', dir: 'ltr'},
    body: {dir: 'ltr'},
    addEventListener(type, handler) {
      (docHandlers[type] ||= []).push(handler);
    },
    dispatchEvent(event) {
      for (const handler of docHandlers[event.type] || []) handler(event);
    }
  };
  const win = {
    go: {main: {App: app}},
    addEventListener() {},
    runtime: {OnFileDrop() {}, EventsOn() {}}
  };
  const context = {
    document, navigator: {language: 'en-US'}, window: win,
    localStorage: {getItem: () => null, setItem() {}},
    setTimeout, clearTimeout,
    console: {warn() {}, error() {}},
    structuredClone
  };
  vm.runInNewContext(source, context, {filename: 'native-app.js'});
  return {elements, rows, app, sent, context, win, document};
}

async function settle() {
  await new Promise(resolve => setImmediate(resolve));
  await new Promise(resolve => setImmediate(resolve));
}

test('check updates offers native-trusted release page and translates into Hebrew', async () => {
  const {elements, sent} = setupUI();
  await settle();
  assert.equal(elements.installedVersion.textContent, 'v0.4.0-beta.3');
  await elements.checkUpdatesBtn.handlers.click();
  assert.equal(elements.updateStatus.dataset.status, 'update_available');
  assert.match(elements.updateStatus.textContent, /0.4.0-beta.4/);
  assert.equal(elements.updateNotice.hidden, false);
  assert.equal(elements.openUpdateBtn.hidden, false);
  await elements.openUpdateBtn.handlers.click();
  assert.deepEqual(sent, [{kind: 'release', tag: 'v0.4.0-beta.4'}]);
  elements.heBtn.handlers.click();
  assert.equal(elements.updateStatus.dataset.status, 'update_available');
  assert.match(elements.updateStatus.textContent, /גרסה חדשה/);
  assert.equal(elements.updateNotice.hidden, false);
});

test('private repository without published releases offers beta artifacts page', async () => {
  const {elements, app, sent} = setupUI();
  await settle();
  app.CheckForUpdates = async () => ({status: 'no_releases'});
  await elements.checkUpdatesBtn.handlers.click();
  assert.equal(elements.updateStatus.dataset.status, 'no_releases');
  assert.match(elements.updateStatus.textContent, /No GitHub Releases/);
  await elements.openUpdateBtn.handlers.click();
  assert.deepEqual(sent, [{kind: 'builds', tag: ''}]);
});

test('authentication and offline errors do not automatically download or install', async () => {
  const {elements, app, sent} = setupUI();
  await settle();
  app.CheckForUpdates = async () => ({status: 'authentication_required'});
  await elements.checkUpdatesBtn.handlers.click();
  assert.match(elements.updateStatus.textContent, /GitHub CLI/);
  await elements.openUpdateBtn.handlers.click();
  assert.deepEqual(sent, [{kind: 'releases', tag: ''}]);

  app.CheckForUpdates = async () => {throw new Error('offline')};
  await elements.checkUpdatesBtn.handlers.click();
  assert.equal(elements.updateStatus.dataset.status, 'error');
  assert.match(elements.updateStatus.textContent, /offline/);
  assert.equal(elements.openUpdateBtn.hidden, true);

  elements.dismissUpdateBtn.handlers.click();
  assert.equal(elements.updateNotice.hidden, true);
  assert.equal(sent.length, 1);
});

test('conversion progress counts completed files only, never fabricated intermediate percentages',async()=>{
  const {elements, context}=setupUI();
  await settle();
  const set=(phase,done,total)=>{
    vm.runInNewContext('progressPhase='+JSON.stringify(phase)+
      '; completedBatchFiles='+done+'; totalBatchFiles='+total+'; renderProgress();',context);
  };
  assert.equal(elements.progressPercent.textContent,'0%');
  set('working',0,3);
  assert.equal(elements.progressPercent.textContent,'0%');
  set('working',1,3);
  assert.equal(elements.progressPercent.textContent,'33%');
  assert.equal(elements.progressFill.style.width,'33%');
  set('working',2,3);
  assert.equal(elements.progressPercent.textContent,'66%');
  set('error',2,3);
  assert.equal(elements.progressPercent.textContent,'66%');
  assert.match(elements.progressDescription.textContent,/stopped/i);
  set('done',3,3);
  assert.equal(elements.progressPercent.textContent,'100%');
  assert.match(elements.progressDescription.textContent,/finished/i);
});

test('invalid preferences require explicit recovery and preserve backup location', async () => {
  const {elements, app, win} = setupUI();
  await settle();
  app.GetSettings = async () => { throw new Error('invalid settings JSON'); };
  let recovered = 0;
  win.confirm = () => false;
  app.RecoverInvalidSettings = async () => {
    recovered++;
    return {settings: {printers: {}}, backupPath: '/backup/settings.invalid.bak'};
  };
  await elements.settingsBtn.handlers.click();
  assert.equal(recovered, 0, 'declining confirmation must not recover');
  assert.equal(elements.settingsDialog.open, false);

  win.confirm = () => true;
  await elements.settingsBtn.handlers.click();
  assert.equal(recovered, 1);
  assert.equal(elements.settingsDialog.open, true);
  assert.match(elements.activityDetail.textContent, /backup\/settings\.invalid\.bak/);
});


test('Rust-native engine selector stays developer-only and turns off with developer mode', async () => {
  const {elements, document} = setupUI();
  await settle();
  assert.equal(elements.rustNativeDevControl.hidden, true, 'installed default hides pilot');
  assert.equal(elements.rustNativeOptIn.checked, false, 'installed default is Go');
  assert.equal(elements.cancelRustConversionBtn.hidden, true);

  document.dispatchEvent({type: 'slicebridge:developer-mode', detail: {enabled: true}});
  assert.equal(elements.rustNativeDevControl.hidden, false);
  elements.rustNativeOptIn.checked = true;
  elements.rustNativeOptIn.handlers.change({target: elements.rustNativeOptIn});
  assert.equal(elements.rustNativeOptIn.checked, true, 'pilot requires an explicit click');

  document.dispatchEvent({type: 'slicebridge:developer-mode', detail: {enabled: false}});
  assert.equal(elements.rustNativeDevControl.hidden, true);
  assert.equal(elements.rustNativeOptIn.checked, false, 'disabling developer mode clears pilot');
  assert.equal(elements.cancelRustConversionBtn.hidden, true);
});
