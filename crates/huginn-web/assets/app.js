(function () {
  'use strict';

  const tbody = document.getElementById('probe-body');
  const placeholder = document.getElementById('placeholder');

  // Probe name → its row element. Keyed on the raw name, because a DOM id cannot
  // carry one losslessly: the id used to be derived by replacing every character
  // outside [A-Za-z0-9_-] with '_', so `db.primary` and `db/primary` collapsed
  // onto the same id and overwrote each other — two configured probes, one row,
  // and nothing saying which one you were looking at. Probe names only have to
  // be non-empty and unique (huginn-core validates exactly that), so both are
  // legal. A Map keyed on the name makes the collision impossible rather than
  // unlikely.
  const rows = new Map();

  function upsertRow(result) {
    // Remove placeholder on first real result
    if (placeholder && placeholder.parentNode) {
      placeholder.parentNode.removeChild(placeholder);
    }

    const isUp = result.up;
    const statusCell = isUp
      ? '<td class="up">✅ UP</td>'
      : '<td class="down">❌ DOWN</td>';
    const err = result.error || '-';
    const ms = result.response_ms != null ? result.response_ms.toFixed(1) + 'ms' : '-';

    const cells =
      '<td>' + escHtml(result.probe_name) + '</td>' +
      '<td>' + escHtml(result.probe_type) + '</td>' +
      '<td>' + escHtml(result.target)     + '</td>' +
      statusCell +
      '<td>' + escHtml(ms)               + '</td>' +
      '<td>' + escHtml(err)              + '</td>';

    let row = rows.get(result.probe_name);
    if (row) {
      row.innerHTML = cells;
    } else {
      row = document.createElement('tr');
      row.innerHTML = cells;
      tbody.appendChild(row);
      rows.set(result.probe_name, row);
    }

    // Brief highlight to indicate the row was updated
    row.classList.remove('updated');
    void row.offsetWidth; // force reflow
    row.classList.add('updated');
  }

  function escHtml(str) {
    return String(str)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;');
  }

  // Open the stream *before* asking for the snapshot, and hold what it sends
  // until the snapshot has been applied. `/events` subscribes at connect time
  // and replays nothing, so opening it second left a window in which a result
  // was shown only at the next tick; applying the snapshot second would let a
  // stale row overwrite a newer one. Buffering closes both directions.
  let seeded = false;
  const pending = [];

  const es = new EventSource('/events');

  es.onmessage = function (evt) {
    let result;
    try {
      result = JSON.parse(evt.data);
    } catch (e) {
      console.warn('hugin: could not parse SSE message', e);
      return;
    }
    if (seeded) {
      upsertRow(result);
    } else {
      pending.push(result);
    }
  };

  es.onerror = function () {
    console.warn('hugin: SSE connection lost, browser will retry automatically');
  };

  // Apply the snapshot, then drain what arrived while it was in flight, in
  // arrival order — so a later event always wins over the snapshot it overtook.
  function seed(results) {
    if (results) {
      Object.values(results).forEach(upsertRow);
    }
    seeded = true;
    pending.splice(0).forEach(upsertRow);
  }

  fetch('/metrics/latest')
    .then(function (r) { return r.json(); })
    .then(seed)
    .catch(function () {
      // No snapshot — the stream alone fills the table. Seed anyway, or the
      // buffered events would sit in `pending` forever.
      seed(null);
    });
}());
