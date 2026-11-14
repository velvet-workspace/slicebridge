const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const os = require('node:os');
const vm = require('node:vm');
const Module = require('node:module');

const handlers = new Map();
let blockedVendorTarget = null;
const fakeElectron = {
  app: {
    isPackaged: false,
    whenReady: () => new Promise(() => {}),
    on: () => {}
  },
  BrowserWindow: class {},
  ipcMain: { handle: (name, callback) => handlers.set(name, callback) },
  dialog: { showOpenDialog: async () => ({ canceled: true, filePaths: [] }) },
  shell: { openPath: async () => '' }
};

function fakeVendorBundle(target, sourceNozzle = '0.4') {
  const targetNozzle = target === 'c5' && sourceNozzle === '0.2' ? '0.25' : sourceNozzle;
  const models = {
    u1: 'Snapmaker U1',
    c5: 'Flashforge Creator 5',
    cc2: 'Elegoo Centauri Carbon 2',
    qidi: 'X-Plus 5'
  };
  return {
    target,
    slicer: 'synthetic test slicer',
    nozzle: targetNozzle,
    profile: {
      name: target,
      display_name: models[target],
      printer_model_id: models[target],
      filament_slots: 4,
      filament_profiles: { PLA: 'Synthetic PLA' },
      default_filament_profile: 'Synthetic PLA',
      project_settings: {
        printer_model: models[target],
        printer_settings_id: 'Synthetic machine 0.4',
        print_settings_id: 'Synthetic Standard 0.2',
        machine_start_gcode: 'G28 ; SYNTHETIC',
        machine_end_gcode: 'M104 S0 ; SYNTHETIC',
        nozzle_diameter: [targetNozzle],
        filament_type: ['PLA'],
        filament_colour: ['#FFFFFF'],
        filament_settings_id: ['Synthetic PLA'],
        layer_height: '0.2',
        enable_support: '0'
      }
    },
    material_settings: {
      PLA: { filament_type: ['PLA'], filament_colour: ['#FFFFFF'],
        filament_density: ['1.24'], nozzle_temperature: ['210'],
        filament_flow_ratio: ['0.98'], filament_max_volumetric_speed: ['18'],
        filament_settings_id: ['Synthetic PLA'] }
    }
  };
}

const originalLoad = Module._load;
try {
  Module._load = function(request, parent, isMain) {
    if (request === 'electron') return fakeElectron;
    if (request === './slicer-profiles' &&
      parent && parent.filename.endsWith(path.sep + 'main.js')) {
      return {
        PROFILES: { u1: {}, c5: {}, cc2: {}, qidi: {} },
        loadTargetBundle: async (target, options = {}) => {
          if (target === blockedVendorTarget) throw new Error('Synthetic missing vendor preset');
          return fakeVendorBundle(target, options.nozzle);
        }
      };
    }
    return originalLoad.call(this, request, parent, isMain);
  };
  require('../desktop/electron/main.js');
} finally {
  Module._load = originalLoad;
}

const fixture = path.join(__dirname, 'fixtures', 'bambu-one-color.3mf');

test('renderer script parses and copy-only conversion is absent', async () => {
  const html = await fs.readFile(path.join(__dirname, '..', 'desktop', 'electron', 'index.html'), 'utf8');
  // The fixture is project-owned static HTML; locate its literal inline
  // script rather than treating a regexp as an HTML sanitizer.
  const start = html.indexOf('<script>');
  const end = html.indexOf('</script>', start + '<script>'.length);
  assert.ok(start !== -1 && end > start, 'renderer script exists');
  assert.doesNotThrow(() => new vm.Script(html.slice(start + '<script>'.length, end)));
  const main = await fs.readFile(path.join(__dirname, '..', 'desktop', 'electron', 'main.js'), 'utf8');
  assert.ok(!main.includes('fs.copyFile(filePath, outputPath)'), 'must not fake conversion');
});

test('IPC reports all four target adapters as available when profiles resolve', async () => {
  const states = await handlers.get('get-target-availability')();
  assert.deepEqual(states.map((x) => x.target), ['u1', 'c5', 'cc2', 'qidi']);
  assert.ok(states.every((x) => x.available));
});

test('IPC converts all 4 targets concurrently, protects source, and handles collisions', async () => {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), '3mf-bridge-ipc-all-'));
  try {
    const input = path.join(dir, 'model.3mf');
    await fs.copyFile(fixture, input);
    const before = await fs.readFile(input);
    const convert = handlers.get('convert-3mf');
    const targets = ['u1', 'c5', 'cc2', 'qidi'];
    const modelPrefixes = {u1:'U1',c5:'Creator5',cc2:'CentauriCarbon2',qidi:'XPlus5'};

    for (let attempt = 1; attempt <= 2; attempt++) {
      const result = await convert(null, { filePath: input, targets });
      assert.equal(result.ok, true, JSON.stringify(result));
      assert.equal(result.results.length, 4, JSON.stringify(result));
      assert.equal(result.failures.length, 0);
      for (const target of targets) {
        const entry = result.results.find((r) => r.target === target);
        assert.ok(entry && entry.report && entry.report.ok);
        assert.equal(entry.prefix, modelPrefixes[target]);
        assert.equal(entry.outputName,
          entry.prefix + '-model' + (attempt === 1 ? '' : '-2') + '.3mf');
        assert.equal(entry.report.engine, 'bl2std');
        assert.ok(entry.report.removed_slice_cache_entries >= 3);
        const out = await fs.readFile(entry.outputPath);
        assert.ok(!out.equals(before), 'must not copy-only for ' + target);
      }
    }
    assert.ok((await fs.readFile(input)).equals(before));
  } finally {
    await fs.rm(dir, { recursive: true, force: true });
  }
});

test('IPC isolates target-specific errors and rejects invalid target keys', async () => {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), '3mf-bridge-errors-'));
  try {
    const input = path.join(dir, 'model.3mf');
    await fs.copyFile(fixture, input);
    let result = await handlers.get('convert-3mf')(null, {
      filePath: input, targets: ['u1', 'invalid']
    });
    assert.equal(result.ok, false);
    assert.equal(result.code, 'TARGET_INVALID');

    result = await handlers.get('convert-3mf')(null, {
      filePath: input, targets: ['qidi', 'qidi']
    });
    assert.equal(result.ok, false);
    assert.equal(result.code, 'TARGET_INVALID');
    assert.deepEqual(await fs.readdir(dir), ['model.3mf']);
  } finally {
    await fs.rm(dir, { recursive: true, force: true });
  }
});

test('IPC rejects malformed .3mf without writing misleading output', async () => {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), '3mf-bridge-invalid-'));
  try {
    const input = path.join(dir, 'broken.3mf');
    await fs.writeFile(input, 'not zip data');
    const result = await handlers.get('convert-3mf')(null, {
      filePath: input, targets: ['u1', 'c5', 'cc2', 'qidi']
    });
    assert.equal(result.ok, false);
    assert.equal(result.failures.length, 4);
    assert.ok(result.failures.every((r) => r.code === 'SOURCE_INVALID'), JSON.stringify(result));
    assert.deepEqual(await fs.readdir(dir), ['broken.3mf']);
  } finally {
    await fs.rm(dir, { recursive: true, force: true });
  }
});

test('one missing vendor profile does not cancel successful conversions for other targets', async () => {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), '3mf-bridge-partial-'));
  blockedVendorTarget = 'cc2';
  try {
    const input = path.join(dir, 'model.3mf');
    await fs.copyFile(fixture, input);
    const result = await handlers.get('convert-3mf')(null, {
      filePath: input, targets: ['u1', 'c5', 'cc2', 'qidi']
    });
    assert.equal(result.ok, true, JSON.stringify(result));
    assert.deepEqual(result.results.map((r) => r.target), ['u1', 'c5', 'qidi']);
    assert.equal(result.failures.length, 1);
    assert.equal(result.failures[0].target, 'cc2');
    assert.equal(result.failures[0].code, 'ENGINE_FAILED');
    const files = await fs.readdir(dir);
    assert.ok(files.includes('U1-model.3mf'));
    assert.ok(files.includes('Creator5-model.3mf'));
    assert.ok(files.includes('XPlus5-model.3mf'));
    assert.ok(!files.includes('CentauriCarbon2-model.3mf'));
  } finally {
    blockedVendorTarget = null;
    await fs.rm(dir, { recursive: true, force: true });
  }
});

test('source 0.2 mm selects Creator 5 native 0.25 mm automatically, other targets remain 0.2', async () => {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), '3mf-bridge-ipc-nozzle-'));
  try {
    const source = path.join(dir, 'fine.3mf');
    await fs.copyFile(path.join(__dirname, 'fixtures', 'bambu-nozzle-0.2.3mf'), source);
    const result = await handlers.get('convert-3mf')(null, {
      filePath: source, targets: ['u1', 'c5', 'cc2', 'qidi']
    });
    assert.equal(result.ok, true, JSON.stringify(result));
    assert.equal(result.results.length, 4);
    assert.equal(result.sourceNozzle, '0.2');
    for (const entry of result.results) {
      assert.equal(entry.report.source_nozzle, '0.2');
      const expected = entry.target === 'c5' ? '0.25' : '0.2';
      assert.equal(entry.report.target_nozzle, expected);
      assert.equal(entry.report.nozzle_remapped, entry.target === 'c5');
    }
  } finally {
    await fs.rm(dir, { recursive: true, force: true });
  }
});

test('source 0.6/0.8 mm selects matching target nozzles for all 4 printers', async () => {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), '3mf-bridge-ipc-wide-'));
  try {
    for (const size of ['0.6', '0.8']) {
      const source = path.join(dir, 'wide-' + size + '.3mf');
      await fs.copyFile(path.join(__dirname, 'fixtures', 'bambu-nozzle-' + size + '.3mf'), source);
      const result = await handlers.get('convert-3mf')(null, {
        filePath: source, targets: ['u1', 'c5', 'cc2', 'qidi']
      });
      assert.equal(result.ok, true, JSON.stringify(result));
      assert.equal(result.results.length, 4);
      assert.ok(result.results.every((r) => r.report.target_nozzle === size));
    }
  } finally {
    await fs.rm(dir, { recursive: true, force: true });
  }
});
