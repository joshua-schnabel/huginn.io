(function () {
  'use strict';

  const tbody = document.getElementById('probe-body');
  const placeholder = document.getElementById('placeholder');

  function upsertRow(result) {
    // Remove placeholder on first real result
    if (placeholder && placeholder.parentNode) {
      placeholder.parentNode.removeChild(placeholder);
    }

    const id = 'probe-' + result.probe_name.replace(/[^a-zA-Z0-9_-]/g, '_');
    let row = document.getElementById(id);

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

    if (row) {
      row.innerHTML = cells;
    } else {
      row = document.createElement('tr');
      row.id = id;
      row.innerHTML = cells;
      tbody.appendChild(row);
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

  // Seed the table with the current snapshot, then open the SSE stream.
  fetch('/metrics/latest')
    .then(function (r) { return r.json(); })
    .then(function (results) {
      Object.values(results).forEach(upsertRow);
    })
    .catch(function () { /* ignore — SSE will fill the table */ });

  var es = new EventSource('/events');

  es.onmessage = function (evt) {
    try {
      upsertRow(JSON.parse(evt.data));
    } catch (e) {
      console.warn('hugin: could not parse SSE message', e);
    }
  };

  es.onerror = function () {
    console.warn('hugin: SSE connection lost, browser will retry automatically');
  };
}());
