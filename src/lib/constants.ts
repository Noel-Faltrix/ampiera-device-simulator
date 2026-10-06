export const LIVE_HOST = "api.ampiera.de";
export const MAX_LIVE_BOXES = 3;
export const FRAME_CAP = 2000;
export const APP_POLL_SECONDS = 30;

export const LIVE_BOX_HINT = `Diese Wallbox verbindet sich mit dem Produktivserver ${LIVE_HOST}. Verwende nur die Kennung einer Simulationsanlage. Es sind höchstens ${MAX_LIVE_BOXES} Live-Wallboxen gleichzeitig verbunden.`;
export const LIVE_BANNER =
  "Live: Diese Wallbox ist mit dem Produktivserver verbunden. Aktionen wirken auf den echten Betrieb.";
export const LIVE_APP_NOTICE = "Du meldest dich am Produktivserver an. Die App-Sicht liest nur.";
