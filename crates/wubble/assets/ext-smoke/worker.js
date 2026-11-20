// --smoke extensions: answers the content script; on `{run: 'apis'}` drives the tab, window and
// action APIs against wubble's tabs and reports what they said.
chrome.runtime.onMessage.addListener((message, sender, reply) => {
  if (message.run === 'apis') {
    apis(sender).then(reply, (e) => reply({ error: String(e && e.message ? e.message : e) }));
    return true;
  }
  reply({ pong: 'worker:' + message.ping });
});

// A click on the action marks the tab it was clicked on (which proves the tab object).
chrome.action.onClicked.addListener((tab) => {
  chrome.action.setBadgeText({ tabId: tab.id, text: 'hit' });
});

async function apis(sender) {
  const current = await chrome.windows.getCurrent();
  const all = await chrome.windows.getAll();
  const last = await chrome.windows.getLastFocused();
  const [active] = await chrome.tabs.query({ active: true, currentWindow: true });
  // Completions seen before tabs.create answers count too.
  const complete = new Set();
  let wake = () => {};
  const onUpdated = (id, info) => {
    if (info.status === 'complete') {
      complete.add(id);
      wake();
    }
  };
  chrome.tabs.onUpdated.addListener(onUpdated);
  const created = await chrome.tabs.create({
    url: new URL('/article?opened', sender.url).href,
    active: false,
  });
  await new Promise((resolve) => {
    wake = () => complete.has(created.id) && resolve();
    wake();
  });
  chrome.tabs.onUpdated.removeListener(onUpdated);
  // A tab created active is activated too (onActivated, seen before tabs.create answers).
  const activated = new Set();
  let wakeActive = () => {};
  const onActivated = (info) => {
    activated.add(info.tabId);
    wakeActive();
  };
  chrome.tabs.onActivated.addListener(onActivated);
  const shown = await chrome.tabs.create({ url: new URL('/article?shown', sender.url).href });
  await new Promise((resolve) => {
    wakeActive = () => activated.has(shown.id) && resolve();
    wakeActive();
  });
  chrome.tabs.onActivated.removeListener(onActivated);
  // No URL: wubble's start page, which has no tab object.
  const start = await chrome.tabs.create({});
  const echo = await chrome.tabs.sendMessage(sender.tab.id, { hello: 'tab' });
  await chrome.action.setBadgeText({ text: '7' });
  return {
    windows: { current: current.id, type: current.type, all: all.map((w) => w.id), last: last.id },
    query: active && { id: active.id, index: active.index, url: active.url, window: active.windowId },
    sender: sender.tab.id,
    created: { id: created.id, index: created.index, active: created.active, window: created.windowId },
    shown: { id: shown.id, index: shown.index, active: shown.active },
    start: start === undefined ? 'none' : 'tab',
    echo: echo && echo.echo,
  };
}
