AdvanceLiberty: Enhanced
Windows x64 / preview

Это неофициальная нативная оболочка для предоставленного игроком образа игры.
Мы не распространяем ROM, BIOS, сохранения или извлечённые игровые ресурсы.
Пользователь должен самостоятельно предоставить законно полученный дамп
собственного картриджа. Приложение не скачивает игру и не предоставляет
права на оригинальную игру. Проект не связан с её правообладателями.

ЗАПУСК
1. Полностью распакуйте папку в доступное для записи место.
2. Положите собственный неизменённый USA ROM (.gba) в папку roms.
   Имя файла может быть любым. Также можно перетащить ROM на EXE.
3. Запустите AdvanceLiberty.exe. Никаких скриптов или компилятора не требуется.

Поддерживаемый SHA-256:
215c574d5afbda0aadbf89dc76b765cfaaeb35953eab63f7687d8bfe8a8e0f4d
Европейские образы и патчи ROM пока не поддерживаются.
При отсутствии/несовместимости ROM появляется понятное сообщение.

ПО УМОЛЧАНИЮ
Enhanced-рендер, Enhanced Audio, современное управление.
Целевой срок жизни денег, оружия и трупов: 30 секунд. При нехватке игровых
ресурсов предметы могут быть удалены раньше; лимиты игровых пулов сохраняются.

УПРАВЛЕНИЕ
Пешком: WASD — движение, Shift — бег, J/ЛКМ — огонь/удар,
Space — прыжок, F — сесть в транспорт, Tab — оружие.
Транспорт: W/S — газ/тормоз/задний ход, A/D — руль, Space — ручник,
Shift — сигнал/сирена, F — выйти, Ctrl+A/D — башня, Caps Lock — спецмиссия.
V — Auto/Low/Medium/High камера. Esc/Enter — игровое меню.
Enhanced Options — графика, язык и переназначение клавиш.
F6 — Enhanced/Original рендер, F8 — Enhanced/Classic управление,
F11 — полный экран. Ctrl+F12 — панель читов.

ДАННЫЕ
data/saves/usa.sav — игровой прогресс, .bak — предыдущая версия.
data/config — настройки. data/logs — журналы запуска.
data/sessions — диагностические записи игровых сессий.
При первом запуске languages/en.json извлекается из собственного ROM.
Дополнительные языковые JSON можно положить в languages и выбрать в меню.
Не копируйте свои ROM, сохранения и записи сессий в публичный архив сборки.

ПАРАМЕТРЫ EXE
--original-audio       прежний аппаратный звук
--drop-lifetime 0      исходные правила удаления предметов/трупов
--no-record            без записи диагностической сессии
--rom "путь к ROM"     использовать файл вне папки сборки

Нативные библиотеки должны оставаться в native/usa. Не заменяйте ими
библиотеки другой версии. Не запускайте две копии с одним сохранением.
Лицензии сторонних компонентов и уведомления находятся в licenses.

ENGLISH QUICK START
Provide your own legally obtained, unmodified USA cartridge dump.
No ROM, BIOS, save data or extracted game assets are included or downloaded.
Extract the complete folder, put your .gba file in roms, then run
AdvanceLiberty.exe. Any ROM filename is accepted; drag and drop also works.
European/patched ROMs are incompatible. Enhanced Audio and 30-second drop
lifetime are enabled by default. Esc opens the menu; Enhanced Options
contains PC preferences. User data stays inside this portable folder.
