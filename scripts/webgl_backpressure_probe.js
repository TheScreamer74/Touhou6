// Evaluate after the actual WASM game is playing at the settled 2D title.
// Mask completion status for1000ms to test backpressure; no pixels are edited.
// The game uses GLES/WebGL2, not GPUQueue/onSubmittedWorkDone browser promises.
// Engine::encode_pass draws6vertices/quad; surface presentation draws3vertices.
// GLES25 lowers both to drawArraysInstanced. Unknown geometry fails closed.
// Execution with Playwright: await page.evaluate(probeSourceText).
(async () => {
  const canvas = document.querySelector('#th06-canvas');
  if (!canvas || !document.body.classList.contains('playing')) {
    return { passed: false, limitation: 'The actual WASM game must be playing' };
  }
  const gl = canvas.getContext('webgl2');
  if (!gl) return { passed: false, limitation: 'WebGL2 context unavailable' };
  const names = ['drawArrays', 'drawArraysInstanced', 'drawElements',
    'drawElementsInstanced', 'fenceSync', 'deleteSync',
    'getSyncParameter', 'clientWaitSync', 'finish'];
  const methods = Object.fromEntries(names.map(name => [name, gl[name]]));
  const descriptors = Object.fromEntries(names.map(name =>
    [name, Object.getOwnPropertyDescriptor(gl, name)]));
  let phase = 'baseline';
  let quadsSinceFence = false;
  let triangleSinceFence = false;
  let unknownSinceFence = false;
  const pending = new Map();
  const counts = Object.fromEntries(['baseline', 'hold', 'resume'].map(name =>
    [name, { historyChains: 0, otherSubmits: 0, unclassifiedSubmits: 0,
      quadDraws: 0, triangleDraws: 0, unknownDraws: 0, heartbeat: 0, finish: 0,
      waits: 0, maxWaitNanoseconds: 0, statusPolls: 0, maxPendingHistory: 0,
      maxPendingAll: 0 }]));
  const sleep = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));
  const heartbeat = setInterval(() => counts[phase].heartbeat++, 20);
  const observeArrays = (mode, first, vertices, instances) => {
    if (mode === gl.TRIANGLES && instances === 1 && vertices >= 6
        && vertices % 6 === 0 && first % 6 === 0) {
      quadsSinceFence = true;
      counts[phase].quadDraws++;
    } else if (mode === gl.TRIANGLES && instances === 1 && first === 0 && vertices === 3) {
      triangleSinceFence = true;
      counts[phase].triangleDraws++;
    } else {
      unknownSinceFence = true;
      counts[phase].unknownDraws++;
    }
  };
  try {
    gl.drawArrays = function (mode, first, vertices) {
      observeArrays(mode, first, vertices, 1);
      return Reflect.apply(methods.drawArrays, this, [mode, first, vertices]);
    };
    gl.drawArraysInstanced = function (mode, first, vertices, instances) {
      observeArrays(mode, first, vertices, instances);
      return Reflect.apply(methods.drawArraysInstanced, this, [mode, first, vertices, instances]);
    };
    gl.drawElements = function (...args) {
      unknownSinceFence = true;
      counts[phase].unknownDraws++;
      return Reflect.apply(methods.drawElements, this, args);
    };
    gl.drawElementsInstanced = function (...args) {
      unknownSinceFence = true;
      counts[phase].unknownDraws++;
      return Reflect.apply(methods.drawElementsInstanced, this, args);
    };
    gl.fenceSync = function (...args) {
      const sync = Reflect.apply(methods.fenceSync, this, args);
      if (sync) {
        const kind = unknownSinceFence ? 'unclassified'
          : quadsSinceFence ? 'history' : triangleSinceFence ? 'surface' : 'unclassified';
        pending.set(sync, kind);
        counts[phase][kind === 'history' ? 'historyChains'
          : kind === 'surface' ? 'otherSubmits' : 'unclassifiedSubmits']++;
        counts[phase].maxPendingAll = Math.max(counts[phase].maxPendingAll, pending.size);
        counts[phase].maxPendingHistory = Math.max(counts[phase].maxPendingHistory,
          [...pending.values()].filter(kind => kind === 'history').length);
      }
      quadsSinceFence = false;
      triangleSinceFence = false;
      unknownSinceFence = false;
      return sync;
    };
    gl.deleteSync = function (sync) {
      pending.delete(sync);
      return Reflect.apply(methods.deleteSync, this, [sync]);
    };
    gl.getSyncParameter = function (sync, parameter) {
      if (parameter === gl.SYNC_STATUS) {
        counts[phase].statusPolls++;
        if (phase === 'hold' && pending.has(sync)) return gl.UNSIGNALED;
      }
      return Reflect.apply(methods.getSyncParameter, this, [sync, parameter]);
    };
    gl.clientWaitSync = function (sync, flags, timeout) {
      counts[phase].waits++;
      counts[phase].maxWaitNanoseconds = Math.max(counts[phase].maxWaitNanoseconds, timeout);
      if (phase === 'hold' && pending.has(sync)) return gl.TIMEOUT_EXPIRED;
      return Reflect.apply(methods.clientWaitSync, this, [sync, flags, timeout]);
    };
    gl.finish = function (...args) {
      counts[phase].finish++;
      return Reflect.apply(methods.finish, this, args);
    };
    await sleep(1000);
    if (counts.baseline.historyChains < 10 || counts.baseline.statusPolls < 10
        || counts.baseline.unknownDraws || counts.baseline.unclassifiedSubmits) {
      return { passed: false, backendObserved: false, counts,
        limitation: 'No unambiguous settled2D title history/fence stream; no backpressure claim' };
    }
    phase = 'hold';
    await sleep(1000);
    const pendingAtRelease = pending.size;
    phase = 'resume';
    await sleep(1000);
    const historyAfterResume = [...pending.values()].filter(kind => kind === 'history').length;
    const backendObserved = counts.baseline.historyChains >= 10
      && counts.baseline.statusPolls >= 10;
    return {
      passed: backendObserved && counts.hold.historyChains >= 1
        && counts.hold.historyChains <= 2 && counts.hold.maxPendingHistory <= 2
        && counts.hold.heartbeat >= 20 && counts.hold.otherSubmits >= 10
        && counts.resume.historyChains >= 10 && historyAfterResume <= 2
        && pending.size <= counts.baseline.maxPendingAll + 4
        && Object.values(counts).every(c => c.finish === 0 && c.maxWaitNanoseconds === 0
          && c.unknownDraws === 0 && c.unclassifiedSubmits === 0),
      backend: 'WebGL2; GLES Normal fences',
      backendObserved,
      counts,
      pendingAtRelease,
      pendingAfterResume: pending.size,
      historyAfterResume,
      method: 'Settled2D title: six-vertex quad drawArrays/Instanced identifies history; isolated three-vertex triangle identifies surface submit. Hold SYNC_STATUS at UNSIGNALED1000ms; observe heartbeat and cleanup after release. Unknown geometry or absent history fails closed.',
      scope: 'Two pending history draw chains, not two total GL submissions. Surface blits continue eachRAF and their fence high-water is reported. This short test does not prove long-term browser GPU memory stability.',
    };
  } finally {
    phase = 'resume';
    clearInterval(heartbeat);
    for (const name of names) {
      if (descriptors[name]) Object.defineProperty(gl, name, descriptors[name]);
      else delete gl[name];
    }
  }
})()
