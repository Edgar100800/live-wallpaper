// ParticleWall shared script: hardening applied at document-end.
(function () {
  document.addEventListener('contextmenu', function (e) { e.preventDefault(); }, true);
  document.addEventListener('selectstart', function (e) { e.preventDefault(); }, true);
  document.addEventListener('dragstart', function (e) { e.preventDefault(); }, true);
  var s = document.createElement('style');
  s.textContent = 'html,body{overflow:hidden !important;margin:0;padding:0;}' +
                  '*{user-select:none !important;-webkit-user-select:none !important;}' +
                  // Canvas may render at reduced resolution (innerWidth patch);
                  // stretch it to full screen regardless of its inline size.
                  'body > canvas{width:100vw !important;height:100vh !important;}';
  (document.head || document.documentElement).appendChild(s);
})();
