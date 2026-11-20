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
