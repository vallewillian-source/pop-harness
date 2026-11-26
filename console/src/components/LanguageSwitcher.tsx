import { LOCALES, useI18n } from "../i18n";
import type { Locale } from "../i18n/types";
import "../styles/topbar.css";

const LOCALE_ORDER: Locale[] = ["pt-BR", "en-US"];

export function LanguageSwitcher() {
    const { locale, setLocale, translate } = useI18n();

    return (
        <div
            className="poph-lang"
            role="group"
            aria-label={translate("common.languageSwitcherLabel")}
        >
            {LOCALE_ORDER.map((item) => {
                const active = locale === item;
                const labelKey =
                    item === "pt-BR" ? "common.languagePortuguese" : "common.languageEnglish";
                const accessibleLabel = translate(labelKey);
                return (
                    <button
                        key={item}
                        type="button"
                        className="poph-lang__btn"
                        aria-pressed={active}
                        aria-label={accessibleLabel}
                        title={accessibleLabel}
                        onClick={() => setLocale(item)}
                    >
                        {LOCALES[item].short}
                    </button>
                );
            })}
        </div>
    );
}
