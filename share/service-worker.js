self.addEventListener('push', event => {
  const message = event.data ? event.data.json() : {};
  event.waitUntil(self.registration.showNotification(message.title || 'Aye, Aye', {
    body: message.body || '', icon: '/icon-192.png'
  }));
});

self.addEventListener('notificationclick', event => {
  event.notification.close();
  event.waitUntil(clients.matchAll({type: 'window', includeUncontrolled: true}).then(windows =>
    windows.length ? windows[0].focus() : clients.openWindow('/')
  ));
});
