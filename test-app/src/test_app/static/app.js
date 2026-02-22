const apiBase = '/api';
const state = {
  table: null,
  limit: 50,
  offset: 0,
  orderBy: null,
  schema: null,
};

const elements = {
  tableSelect: document.getElementById('tableSelect'),
  refreshBtn: document.getElementById('refreshBtn'),
  schema: document.getElementById('schema'),
  columnHeaders: document.getElementById('columnHeaders'),
  tableBody: document.getElementById('tableBody'),
  rowForm: document.getElementById('rowForm'),
  createBtn: document.getElementById('createBtn'),
  updateBtn: document.getElementById('updateBtn'),
  deleteBtn: document.getElementById('deleteBtn'),
  pkInput: document.getElementById('pkInput'),
  limitInput: document.getElementById('limitInput'),
  offsetInput: document.getElementById('offsetInput'),
  orderInput: document.getElementById('orderInput'),
  applyFilters: document.getElementById('applyFilters'),
  status: document.getElementById('status'),
};

function setStatus(message, tone = 'info') {
  const colors = {
    info: 'text-slate-600',
    success: 'text-emerald-600',
    error: 'text-rose-600',
  };
  elements.status.textContent = message;
  elements.status.className = `text-sm ${colors[tone] ?? colors.info}`;
}

async function request(method, url, payload) {
  const options = { method, headers: { 'Content-Type': 'application/json' } };
  if (payload) {
    options.body = JSON.stringify(payload);
  }

  const response = await fetch(`${apiBase}${url}`, options);
  if (!response.ok) {
    let message = 'Request failed';
    try {
      const data = await response.json();
      if (typeof data === 'string') {
        message = data;
      } else if (data?.detail) {
        message = Array.isArray(data.detail) ? data.detail[0].msg ?? JSON.stringify(data.detail) : data.detail;
      }
    } catch {
      message = await response.text();
    }
    throw new Error(message || 'Request failed');
  }
  return response.json();
}

async function fetchTables() {
  const data = await request('GET', '/tables');
  return data.tables;
}

async function fetchSchema(tableName) {
  return request('GET', `/tables/${tableName}/schema`);
}

async function fetchRows(tableName, { limit, offset, orderBy }) {
  const params = new URLSearchParams();
  params.set('limit', limit);
  params.set('offset', offset);
  if (orderBy) {
    params.set('order_by', orderBy);
  }
  return request('GET', `/tables/${tableName}/rows?${params.toString()}`);
}

async function createRow(tableName, payload) {
  return request('POST', `/tables/${tableName}/rows`, payload);
}

async function updateRow(tableName, pk, payload) {
  return request('PUT', `/tables/${tableName}/rows/${pk}`, payload);
}

async function deleteRow(tableName, pk) {
  return request('DELETE', `/tables/${tableName}/rows/${pk}`);
}

function renderTableOptions(tables) {
  elements.tableSelect.innerHTML = tables
    .map((table) => `<option value="${table}">${table}</option>`)
    .join('');
}

function renderSchema(schema) {
  elements.schema.innerHTML = schema.columns
    .map(
      (column) => `
        <div class="border rounded-lg px-3 py-2">
          <p class="font-semibold">${column.name}${column.primary_key ? ' · PK' : ''}</p>
          <p class="text-xs text-slate-500">${column.type}</p>
        </div>
      `,
    )
    .join('');
}

function renderRows(data) {
  const firstRow = data.items[0] || {};
  const columnNames = Object.keys(firstRow);

  if (columnNames.length === 0) {
    elements.columnHeaders.innerHTML = '';
    elements.tableBody.innerHTML = `
      <tr>
        <td class="px-4 py-4 text-center text-slate-500 border-t" colspan="1">
          No rows returned. Try inserting data.
        </td>
      </tr>`;
    return;
  }

  elements.columnHeaders.innerHTML = columnNames
    .map((name) => `<th class="px-4 py-2 text-left text-xs uppercase text-slate-500">${name}</th>`)
    .join('');
  elements.tableBody.innerHTML = data.items
    .map(
      (row) => `
        <tr class="hover:bg-slate-50">
          ${columnNames
            .map((column) => `<td class="px-4 py-2 border-t text-sm">${row[column] ?? ''}</td>`)
            .join('')}
        </tr>
      `,
    )
    .join('');
}

function initForm(schema) {
  state.schema = schema;
  elements.rowForm.innerHTML = schema.columns
    .filter((column) => !column.primary_key)
    .map(
      (column) => `
        <label class="block text-sm font-medium text-slate-700">${column.name}
          <input type="text" name="${column.name}" class="mt-1 block w-full border rounded-md px-3 py-2 focus:border-blue-600 focus:ring-1 focus:ring-blue-200" />
        </label>
      `,
    )
    .join('');
}

function serializeForm() {
  const formData = new FormData(elements.rowForm);
  const payload = {};
  for (const [key, value] of formData.entries()) {
    if (value !== '') {
      payload[key] = value;
    }
  }
  return payload;
}

async function refresh(tableName = state.table) {
  if (!tableName) return;

  setStatus(`Loading ${tableName} metadata…`);
  const schema = await fetchSchema(tableName);
  renderSchema(schema);
  initForm(schema);

  setStatus(`Loading ${tableName} rows…`);
  const rows = await fetchRows(tableName, state);
  renderRows(rows);
  setStatus(`Loaded ${rows.count} row(s).`, 'success');
}

async function bootstrap() {
  try {
    setStatus('Loading tables…');
    const tables = await fetchTables();
    if (tables.length === 0) {
      setStatus('No tables detected. Populate the database first.', 'error');
      return;
    }
    renderTableOptions(tables);
    state.table = tables[0];
    elements.tableSelect.value = state.table;
    await refresh();
  } catch (error) {
    console.error(error);
    setStatus(error.message, 'error');
    alert(error.message);
  }
}

function registerEvents() {
  elements.tableSelect.addEventListener('change', async (event) => {
    state.table = event.target.value;
    state.offset = 0;
    await refresh();
  });

  elements.refreshBtn.addEventListener('click', async () => {
    await refresh();
  });

  elements.applyFilters.addEventListener('click', async () => {
    state.limit = Number(elements.limitInput.value || 50);
    state.offset = Number(elements.offsetInput.value || 0);
    state.orderBy = elements.orderInput.value || null;
    await refresh();
  });

  elements.createBtn.addEventListener('click', async (event) => {
    event.preventDefault();
    try {
      const payload = serializeForm();
      if (Object.keys(payload).length === 0) {
        setStatus('Provide at least one column before creating.', 'error');
        return;
      }
      await createRow(state.table, payload);
      elements.rowForm.reset();
      setStatus('Row created.', 'success');
      await refresh();
    } catch (error) {
      console.error(error);
      setStatus(error.message, 'error');
      alert(error.message);
    }
  });

  elements.updateBtn.addEventListener('click', async (event) => {
    event.preventDefault();
    const pk = elements.pkInput.value.trim();
    if (!pk) {
      setStatus('Provide a primary key value to update.', 'error');
      return;
    }
    try {
      const payload = serializeForm();
      if (Object.keys(payload).length === 0) {
        setStatus('Provide at least one column to update.', 'error');
        return;
      }
      await updateRow(state.table, pk, payload);
      setStatus(`Row ${pk} updated.`, 'success');
      await refresh();
    } catch (error) {
      console.error(error);
      setStatus(error.message, 'error');
      alert(error.message);
    }
  });

  elements.deleteBtn.addEventListener('click', async (event) => {
    event.preventDefault();
    const pk = elements.pkInput.value.trim();
    if (!pk) {
      setStatus('Provide a primary key value to delete.', 'error');
      return;
    }
    if (!confirm(`Delete row ${pk}?`)) {
      return;
    }
    try {
      await deleteRow(state.table, pk);
      setStatus(`Row ${pk} deleted.`, 'success');
      elements.pkInput.value = '';
      await refresh();
    } catch (error) {
      console.error(error);
      setStatus(error.message, 'error');
      alert(error.message);
    }
  });
}

registerEvents();
bootstrap();
