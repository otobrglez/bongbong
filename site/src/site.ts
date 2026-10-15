// What the site's pages share: where the game lives besides this page, and
// how a visitor reaches its maker. Read by the pages' frontmatter at build
// time; nothing here runs in the browser.

/** The game's App Store product page (App Store Connect's record, Apple ID
 *  6816242262). */
export const APP_STORE_URL = "https://apps.apple.com/app/id6816242262";

/** Whether the app is out on the App Store. Until it is, the product page
 *  does not exist, so production shows no badge and no link to it - only
 *  "coming soon". Set it the day the app is released (BB-114). */
const APP_STORE_RELEASED = false;

/** Whether the pages show the App Store badge and links: once the app is
 *  released, and always in a PR preview (`PUBLIC_BB_PREVIEW`, set by
 *  .github/actions/build-web for pr-preview.yml), so the design can be
 *  reviewed before it is. */
export const SHOW_APP_STORE = APP_STORE_RELEASED || import.meta.env.PUBLIC_BB_PREVIEW === "1";

const REPO = "https://github.com/otobrglez/bongbong";

export const SOURCE_URL = REPO;
export const ISSUES_URL = `${REPO}/issues/new`;
export const RELEASES_URL = `${REPO}/releases`;
export const CHANGELOG_URL = `${REPO}/blob/master/CHANGELOG.md`;

/** A file of the latest release, by the name cargo-dist and the platform
 *  workflows give it (the same on every release). */
const latest = (asset: string) => `${RELEASES_URL}/latest/download/${asset}`;

/** The downloads the latest release carries, in the order the page lists
 *  them, each with the sprite its card shows (site/public/art/). */
export const DOWNLOADS = [
  { platform: "Android", file: ".apk", href: latest("bongbong-aarch64-linux-android.apk"), sprite: "tank-p2" },
  { platform: "macOS", file: ".dmg", href: latest("bongbong-macos.dmg"), sprite: "tank-p3" },
  { platform: "Windows", file: ".zip", href: latest("bongbong-x86_64-pc-windows-msvc.zip"), sprite: "tank-p4" },
  { platform: "Linux", file: ".tar.xz", href: latest("bongbong-x86_64-unknown-linux-gnu.tar.xz"), sprite: "tank-enemy" },
];

export const CONTACT_EMAIL = "oto@bongbong.io";

/** The newsletter's Kit form (uid d4592cd03e). The page posts to it as a
 *  plain HTML form, so Kit is reached only by a visitor who subscribes -
 *  no script of Kit's loads with the page. */
export const KIT_FORM_ACTION = "https://app.kit.com/forms/9842743/subscriptions";
