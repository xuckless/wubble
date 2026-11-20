// --smoke extensions: answers the content script.
chrome.runtime.onMessage.addListener((message, _sender, reply) => {
  reply({ pong: 'worker:' + message.ping });
});
