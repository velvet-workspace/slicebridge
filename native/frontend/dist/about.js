'use strict';

// Lightweight, offline About dialog. The installed application's embedded
// license/credit documents are authoritative; no GitHub fetch is needed.
(() => {
  const el = id => document.getElementById(id);
  const dialog = el('aboutDialog');
  if (!dialog) return;

  const translations = {
    he: {
      open:'אודות', title:'אודות SliceBridge',
      subtitle:'פרויקט אישי בקוד פתוח לחיבור בין סלייסרים.',
      version:'גרסה מותקנת', license:'רישיון וקוד פתוח',
      licenseSummary:'SliceBridge מופץ תחת GPL-3.0-only. הרישיון המלא מצורף לתוכנה.',
      credits:'תודות וקרדיטים',
      creditBl:'bl2std — מנוע המרה, GPLv3.',
      creditWails:'Wails — ממשק שולחני מקורי, MIT.',
      creditVendor:'פרופילי מדפסות נטענים מהסלייסרים שמותקנים אצל המשתמש; הם לא מצורפים לתוכנה.',
      fullLicense:'הצגת הרישיון המלא',
      fullCredits:'כל הקרדיטים וההודעות',
      hideDocument:'הסתר מסמך',
      source:'קוד המקור', releases:'גרסאות להורדה',
      offline:'פרטי הרישוי והקרדיטים זמינים גם ללא חיבור לאינטרנט. בדיקת עדכונים וקישורים ל־GitHub דורשים חיבור.',
      close:'סגירה', loading:'טוען מסמך מקומי…',
      unavailable:'המידע אינו זמין כרגע.', linkError:'לא ניתן לפתוח את הקישור.',
      devTitle:'מצב מפתחים',
      devWarning:'התקנה ידנית של תוסף לא ארוז עלולה להיות מסובכת. התוסף אינו מופיע בחנות. מיועד למשתמשים מתקדמים בלבד.',
      devButton:'מצב מפתחים', devOn:'פעיל', devOff:'כבוי',
      devError:'לא ניתן לשנות את מצב המפתחים כרגע.',
    },
    en: {
      open:'About', title:'About SliceBridge',
      subtitle:'A small, open-source project connecting slicers.',
      version:'Installed version', license:'License & open source',
      licenseSummary:'SliceBridge is licensed under GPL-3.0-only. The full license is bundled with the app.',
      credits:'Credits',
      creditBl:'bl2std — conversion engine, GPLv3.',
      creditWails:'Wails — native desktop framework, MIT.',
      creditVendor:'Printer presets are read from your installed slicers, not shipped with the app.',
      fullLicense:'Read the full license',
      fullCredits:'All credits & notices',
      hideDocument:'Hide document',
      source:'View source', releases:'Download builds',
      offline:'License and credit information works offline. Update checks and GitHub links need internet.',
      close:'Close', loading:'Reading bundled document…',
      unavailable:'This information is not available.', linkError:'Could not open the link.',
      devTitle:'Developer mode',
      devWarning:'Manual unpacked extension setup can be tricky. The extension is not listed in a store. For advanced users only.',
      devButton:'Developer mode', devOn:'On', devOff:'Off',
      devError:'Could not change developer mode right now.',
    }
  };

  const api = () => window.go?.main?.App || null;
  const lang = () => document.documentElement.lang === 'he' ? 'he' : 'en';
  const t = key => translations[lang()][key] || key;
  const doc = el('aboutLegalText');
  const legalStatus = el('aboutLegalStatus');
  const linkStatus = el('aboutLinkStatus');
  let shownDocument = null;
  const developerBtn = el('developerModeBtn');
  const developerState = el('developerModeState');
  const developerStatus = el('developerModeStatus');
  // Native state is the only source of truth; anything but an explicit true is locked.
  let developerMode = false;
  let developerSettled = false;

  function renderDeveloperMode() {
    developerBtn.setAttribute('aria-pressed', String(developerMode));
    developerState.textContent = t(developerMode ? 'devOn' : 'devOff');
  }

  function publishDeveloperMode() {
    renderDeveloperMode();
    document.dispatchEvent(new CustomEvent('slicebridge:developer-mode', {detail: {enabled: developerMode}}));
  }

  function render() {
    dialog.dir = lang() === 'he' ? 'rtl' : 'ltr';
    el('aboutBtnLabel').textContent = t('open');
    el('aboutBtn').setAttribute('aria-label', t('open'));
    el('aboutBtn').title = t('open');
    dialog.querySelectorAll('[data-about-i18n]').forEach(node => {
      node.textContent = t(node.dataset.aboutI18n);
    });
    for (const [kind, id] of [['license', 'viewLicenseBtn'], ['credits', 'viewCreditsBtn']]) {
      el(id).textContent = shownDocument === kind ? t('hideDocument') : t(kind === 'license' ? 'fullLicense' : 'fullCredits');
    }
    renderDeveloperMode();
  }

  function close() {
    dialog.close();
  }

  async function showDocument(kind) {
    if (shownDocument === kind) {
      shownDocument = null;
      doc.textContent = '';
      doc.hidden = true;
      legalStatus.textContent = '';
      render();
      return;
    }
    shownDocument = kind;
    doc.hidden = false;
    doc.textContent = t('loading');
    legalStatus.textContent = '';
    render();
    try {
      if (!api()?.GetAboutLegalText) throw new Error('Unavailable');
      const content = await api().GetAboutLegalText(kind);
      if (shownDocument === kind) {
        doc.textContent = typeof content === 'string' && content.trim() ? content : t('unavailable');
      }
    } catch {
      if (shownDocument === kind) {
        doc.textContent = '';
        doc.hidden = true;
        legalStatus.textContent = t('unavailable');
      }
    }
  }

  async function openPage(kind) {
    linkStatus.textContent = '';
    try {
      if (!api()?.OpenAboutPage) throw new Error('Unavailable');
      await api().OpenAboutPage(kind);
    } catch {
      linkStatus.textContent = t('linkError');
    }
  }

  // The Wails bridge can initialise after this script runs; wait briefly, then fail closed.
  async function loadDeveloperMode() {
    for (let i = 0; i < 30 && !api()?.GetDeveloperMode; i++) {
      await new Promise(resolve => setTimeout(resolve, 70));
    }
    let enabled = false;
    try {
      enabled = (await api().GetDeveloperMode()) === true;
    } catch {
      enabled = false;
    }
    if (!developerSettled) {
      developerMode = enabled;
      publishDeveloperMode();
    }
  }

  async function toggleDeveloperMode() {
    developerSettled = true;
    developerStatus.textContent = '';
    developerBtn.disabled = true;
    const requested = !developerMode;
    try {
      if (!api()?.SetDeveloperMode) throw new Error('Unavailable');
      developerMode = (await api().SetDeveloperMode(requested)) === true;
      if (developerMode !== requested) developerStatus.textContent = t('devError');
    } catch {
      developerMode = false;
      developerStatus.textContent = t('devError');
    } finally {
      developerBtn.disabled = false;
      publishDeveloperMode();
    }
  }

  el('aboutBtn').addEventListener('click', () => {
    render();
    if (!dialog.open) dialog.showModal();
  });
  developerBtn.addEventListener('click', toggleDeveloperMode);
  el('closeAboutBtn').addEventListener('click', close);
  el('doneAboutBtn').addEventListener('click', close);
  el('viewLicenseBtn').addEventListener('click', () => showDocument('license'));
  el('viewCreditsBtn').addEventListener('click', () => showDocument('credits'));
  el('aboutSourceBtn').addEventListener('click', () => openPage('source'));
  el('aboutReleasesBtn').addEventListener('click', () => openPage('releases'));
  dialog.addEventListener('close', () => {
    shownDocument = null;
    doc.textContent = '';
    doc.hidden = true;
    legalStatus.textContent = '';
    linkStatus.textContent = '';
    render();
  });
  for (const code of ['he', 'en']) {
    el(code + 'Btn').addEventListener('click', render);
  }
  render();
  loadDeveloperMode();
})();
