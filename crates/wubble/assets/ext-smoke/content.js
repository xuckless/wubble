// --smoke extensions: marks the page with this extension's id, then writes its service worker's
// answer into the mark (the page's own world reads the attribute).
const name = 'data-wubble-ext-' + chrome.runtime.id;
document.documentElement.setAttribute(name, 'loaded');
chrome.runtime.sendMessage({ ping: 'wubble' }, (answer) => {
  const error = chrome.runtime.lastError;
  document.documentElement.setAttribute(
    name,
    answer && answer.pong ? answer.pong : 'error ' + (error ? error.message : 'no answer'),
  );
});

// The worker's tabs.sendMessage.
chrome.runtime.onMessage.addListener((message, _sender, reply) => {
  if (message.hello) {
    reply({ echo: message.hello });
  }
});

// On `?apis=<this id>` (a patched CEF), the worker drives the tab and window APIs; its report
// goes into `data-wubble-apis`.
if (new URLSearchParams(location.search).get('apis') === chrome.runtime.id) {
  chrome.runtime.sendMessage({ run: 'apis' }, (report) => {
    const error = chrome.runtime.lastError;
    document.documentElement.setAttribute(
      'data-wubble-apis',
      JSON.stringify(report || { error: error ? error.message : 'no answer' }),
    );
  });
}
