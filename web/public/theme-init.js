// Sets <html data-theme> before the first paint (docs/ARCHITECTURE.md §9.2). It is a blocking external script in
// <head> because the SPA CSP allows no inline script (§11). localStorage['quire.theme'] holds 'light', 'dark' or
// 'system' (the values of the API's Theme); anything else, or no storage, means 'system', which styles/tokens.css
// resolves with prefers-color-scheme. The server copy (users.theme) wins once /api/v1/auth/me has loaded.
(function () {
  var theme = 'system';
  try {
    var stored = window.localStorage.getItem('quire.theme');
    if (stored === 'light' || stored === 'dark' || stored === 'system') theme = stored;
  } catch (e) {
    // Storage disabled: keep 'system'.
  }
  document.documentElement.setAttribute('data-theme', theme);
})();
