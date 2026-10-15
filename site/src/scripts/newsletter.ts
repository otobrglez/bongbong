// The newsletter's form (src/components/NewsletterForm.astro), submitted in
// place: the form is a plain POST to Kit's form endpoint, which allows any
// origin, so the page sends the same fields with `fetch` and says how it
// went under the field instead of leaving for Kit's confirmation page - on
// the game page, leaving would end the round. Without this script the
// plain POST still works.

import { t } from "./strings";

export function installNewsletter(): void {
  for (const form of document.querySelectorAll<HTMLFormElement>("form[data-newsletter]")) {
    const button = form.querySelector<HTMLButtonElement>("button[type=submit]");
    const row = form.querySelector<HTMLElement>(".row");
    const status = form.querySelector<HTMLElement>(".status");

    const say = (text: string) => {
      if (!status) return;
      status.textContent = text;
      status.hidden = false;
    };

    form.addEventListener("submit", async (e) => {
      e.preventDefault();
      if (button) button.disabled = true;
      try {
        const response = await fetch(form.action, {
          method: "POST",
          body: new FormData(form),
          headers: { Accept: "application/json" },
        });
        const answer = (await response.json().catch(() => ({}))) as { status?: string };
        if (!response.ok || answer.status === "failed") throw new Error(`Kit answered ${response.status}`);
        if (row) row.hidden = true;
        say(t("newsDone"));
      } catch (error) {
        console.error("newsletter:", error);
        say(t("newsFailed"));
        if (button) button.disabled = false;
      }
    });
  }
}
