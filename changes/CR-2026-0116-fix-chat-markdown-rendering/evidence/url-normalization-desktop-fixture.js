const projectPath = '/tmp/spark-markdown-desktop-fixture';
const timestamp = '2026-09-10T00:00:00Z';
const content = '[Desktop HTTPS smoke](hTtPs://EXAMPLE.COM/?spark-markdown-https=CR-2026-0116-normalized)\n\n[Desktop HTTP smoke](hTtP://EXAMPLE.COM/?spark-markdown-http=CR-2026-0116-normalized)\n\n[Local file](/tmp/example.ts:12)\n\n[Unsafe](javascript:alert%281%29)';
const project = { project_id: 'markdown', project_path: projectPath, display_name: 'Markdown desktop smoke', created_at: timestamp, last_opened_at: timestamp, last_accessed_at: timestamp, is_favorite: false, active_conversation_id: 'markdown' };
const conversation = { schema_version: 4, revision: 1, conversation_id: 'markdown', project_path: projectPath, title: 'Markdown desktop smoke', created_at: timestamp, updated_at: timestamp, chat_mode: 'chat', turns: [{ id: 'assistant', role: 'assistant', content: '', timestamp, status: 'complete', kind: 'message' }], segments: [{ id: 'segment', turn_id: 'assistant', order: 1, kind: 'assistant_message', role: 'assistant', status: 'complete', timestamp, updated_at: timestamp, content }], event_log: [], flow_run_requests: [], flow_launches: [] };
localStorage.setItem('spark.ui_route_state', JSON.stringify({ viewMode: 'projects', activeProjectPath: projectPath, activeFlow: null }));
const originalFetch = window.fetch.bind(window);
window.fetch = (input, init) => {
 const url = new URL(typeof input === 'string' ? input : input.url, location.href);
 let data;
 if (url.pathname === '/workspace/api/projects') data = [project];
 if (url.pathname === '/workspace/api/projects/conversations') data = [conversation];
 if (url.pathname === '/workspace/api/projects/metadata') data = { name: 'Markdown', directory: projectPath, branch: 'smoke', commit: 'smoke' };
 if (url.pathname === '/workspace/api/conversations/markdown') data = conversation;
 if (url.pathname === '/workspace/api/projects/state') data = project;
 return data === undefined ? originalFetch(input, init) : Promise.resolve(new Response(JSON.stringify(data), { headers: {'Content-Type':'application/json'} }));
};
window.EventSource = class { readyState = 1; addEventListener() {} removeEventListener() {} close() {} };
