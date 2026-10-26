# Локальные фикстуры регрессионных тестов

[English](README.md) | **Русский**

Этот каталог предназначен для приватных записей, необходимых проверкам с
настоящим ROM. Данные не включаются в Git или пользовательский ZIP.
Обычные unit-тесты от них не зависят.

`menu-session/initial.sav` и `menu-session/input.gba-input` — минимальная
пара записанного запуска для проверок нативных Camera/Save and Quit.
Маршрут должен приводить в меню паузы на кадре 850. Полные дампы,
изображения и журналы для этих двух тестов не нужны.

Другой каталог с этой парой задаётся через `ADVANCE_LIBERTY_MENU_SESSION`.
Сами тесты дополнительно требуют собственного USA ROM в `roms/usa.gba` и
собранных DLL из `native/usa/manifest.json`:

```powershell
cargo test --release --offline --locked --package advance-liberty --bin AdvanceLiberty real_native_camera_roundtrip_resume_and_quit -- --ignored
cargo test --release --offline --locked --package advance-liberty --bin AdvanceLiberty real_native_save_and_exit_writes_and_reloads_progress -- --ignored
```

Записи сессий, которые нужны CollectNative, остаются отдельно в
`data/sessions/`. Сбор покрытия не читает этот каталог.
