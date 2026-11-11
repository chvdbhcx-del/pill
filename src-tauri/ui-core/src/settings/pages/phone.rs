//! Page: Phone (Pill Phone link, docs/phone-protocol.md). The switch starts the server on the PC (`phone.enabled`); the three custom widgets
//! show the pairing QR, the paired phones and the apps seen in notifications; the rest are plain rows.
use crate::settings::schema::*;
use crate::settings::tr::tr;
use serde_json::json;

pub fn phone() -> Page {
    let on = Cond::On("phone.enabled");
    page(
        "phone",
        "phone",
        tr("Телефон", "Phone"),
        vec![
            group_note(
                tr("Связь с телефоном", "Phone link"),
                tr(
                    "В домашней или офисной сети соединение идёт без шифрования (нет TLS): Pill слушает частный адрес и отвергает чужие. Каждый телефон получает свой ключ при подключении; его можно отозвать. Выключено: ничего не слушает и не работает.",
                    "On your home or office network the connection is NOT encrypted (no TLS): Pill listens on a private address and turns everything else away. Each phone gets its own key when it pairs, and you can revoke it. Off: nothing listens and nothing runs.",
                ),
                vec![
                    toggle(
                        "phone.enabled",
                        tr("Связь с телефоном", "Phone link"),
                        tr("Уведомления телефона на острове, ответы на них и файлы между телефоном и компьютером (приложение Pill Phone)", "Your phone's notifications on the island, replies to them and files between phone and PC (the Pill Phone app)"),
                        false,
                    )
                    .kw(&["phone", "android", "pair", "qr", "link", "телефон", "андроид", "связь", "подключить"]),
                    custom("phone.link", tr("Подключить телефон", "Pair a phone"), tr("Наведите камеру телефона на QR-код: откроется страница, где можно скачать приложение или открыть его", "Point the phone's camera at the QR code: a page opens where you can get the app or open it"), "phone.link")
                        .kw(&["qr", "code", "pair", "pairing", "код", "подключение", "приложение", "app"]),
                    custom("phone.devices", tr("Подключённые телефоны", "Paired phones"), tr("Когда телефон был на связи; «Отозвать» закрывает ему доступ сразу", "When each phone was last in touch; Revoke shuts it out at once"), "phone.devices")
                        .kw(&["revoke", "device", "paired", "отозвать", "устройство", "телефоны"]),
                ],
            ),
            group_note(
                tr("Уведомления", "Notifications"),
                tr("Приложения появляются здесь, когда пришло их первое уведомление. Приглушённые не показываются на острове и не попадают в список.", "Apps appear here after their first notification. Muted ones are not shown on the island or in the list."),
                vec![custom("phone.apps", tr("Приложения", "Apps"), tr("Нажмите на приложение, чтобы приглушить его или вернуть", "Click an app to mute it or bring it back"), "phone.apps").kw(&["mute", "apps", "приглушить", "приложения", "telegram"])],
            ),
            group(
                tr("Файлы", "Files"),
                vec![
                    text(
                        "phone.downloadDir",
                        tr("Папка для файлов с телефона", "Folder for files from the phone"),
                        tr("Пусто: Загрузки\\Pill", "Empty: Downloads\\Pill"),
                        TextKind::Path,
                        tr("%USERPROFILE%\\Downloads\\Pill", "%USERPROFILE%\\Downloads\\Pill"),
                        "",
                    )
                    .enabled(on.clone())
                    .kw(&["folder", "download", "папка", "загрузки", "файлы"]),
                    Row::new("phone.maxFileMB", "phone.maxFileMB", tr("Предел размера файла, МБ", "File size cap, MB"), Control::Number { min: 1, max: 1_048_576 }, json!(4096))
                        .hint(tr("Файл больше предела отклоняется сразу", "A bigger file is turned down at once"))
                        .enabled(on.clone())
                        .kw(&["size", "limit", "размер", "лимит", "мб"]),
                    toggle("phone.autoAccept", tr("Принимать файлы с телефона сразу", "Accept files from the phone at once"), tr("Выключено: каждый файл ждёт вашего решения на вкладке «Телефон»", "Off: every file waits for your decision on the Phone tab"), true)
                        .enabled(on.clone())
                        .kw(&["auto", "accept", "автоматически", "принимать"]),
                    text(
                        "phone.apkUrl",
                        tr("Адрес приложения Pill Phone", "Pill Phone app address"),
                        tr("Куда ведёт кнопка «Скачать приложение» на странице QR-кода", "Where the \"Download the app\" button of the QR page leads"),
                        TextKind::Url,
                        tr("https://…/PillPhone.apk", "https://…/PillPhone.apk"),
                        "https://github.com/chvdbhcx-del/pill/releases/download/phone-latest/PillPhone.apk",
                    )
                    .enabled(on.clone())
                    .kw(&["apk", "download", "url", "ссылка", "скачать"]),
                ],
            ),
            group_note(
                tr("Через интернет (ретранслятор)", "Over the internet (relay)"),
                tr(
                    "Через ретранслятор ваши данные сквозно зашифрованы; ретранслятор (Cloudflare Worker, который принадлежит вам) видит только то, что два устройства обмениваются данными, когда и сколько. Телефон, подключённый до этого обновления, нужно подключить заново, чтобы получить ключи.",
                    "Through the relay your data is end-to-end encrypted; the relay (a Cloudflare Worker you own) sees only that two devices exchange data, when, and how much. A phone paired before this update has to be paired again to get its keys.",
                ),
                vec![
                    toggle(
                        "phone.relay",
                        tr("Связь через интернет", "Reach phones over the internet"),
                        tr("Когда телефон не дома, Pill держит одно исходящее соединение с ретранслятором. Без адреса ретранслятора ничего не происходит", "When the phone is away from home Pill keeps one outgoing connection to the relay. Without a relay address nothing happens"),
                        true,
                    )
                    .enabled(on.clone())
                    .kw(&["relay", "internet", "remote", "away", "ретранслятор", "интернет", "вне дома"]),
                    text(
                        "phone.relayUrl",
                        tr("Адрес ретранслятора", "Relay address"),
                        tr("Например wss://pill-relay.ваш-аккаунт.workers.dev; новые подключения телефонов получают этот адрес", "For example wss://pill-relay.your-account.workers.dev; phones paired from now on are given this address"),
                        TextKind::Url,
                        tr("wss://…", "wss://…"),
                        "",
                    )
                    .enabled(Cond::All(vec![on.clone(), Cond::On("phone.relay")]))
                    .kw(&["relay", "url", "worker", "cloudflare", "ретранслятор", "адрес"]),
                    Row::new("phone.relayMaxFileMB", "phone.relayMaxFileMB", tr("Предел файла через интернет, МБ", "File size cap over the internet, MB"), Control::Number { min: 1, max: 4096 }, json!(100))
                        .hint(tr("Больший файл через ретранслятор не передаётся; по домашней сети предел обычный", "A bigger file does not travel through the relay; on the home network the normal cap applies"))
                        .enabled(Cond::All(vec![on, Cond::On("phone.relay")]))
                        .kw(&["relay", "size", "limit", "размер", "лимит", "мб"]),
                ],
            ),
        ],
    )
}
