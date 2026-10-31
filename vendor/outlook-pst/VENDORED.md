# outlook-pst (копия)

Исходный код [outlook-pst 1.2.0](https://crates.io/crates/outlook-pst) (Microsoft, лицензия MIT,
см. LICENSE) для импорта почты из Outlook. Подключается через `[patch.crates-io]`.

Изменения относительно оригинала:

- `src/ltp/heap.rs`: при чтении не проверяется поле `cFree` в HNPAGEMAP. Многие PST,
  созданные конвертерами и старыми программами, пишут туда 0, и оригинал отказывается
  читать такие письма («Invalid HNPAGEMAP cFree»). Значение используется только при записи.
- Удалены примеры (`examples/`) и их dev-зависимости.
