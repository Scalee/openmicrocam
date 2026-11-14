//! The menu's tables: its sections, the items in each, what an item is (a number on a slider, a value, an action),
//! which camera control it shows, its name and its description.

use crate::text::T;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Picture,
    Exposure,
    Overlays,
    Recording,
    Setup,
    System,
}

/// A row of the menu's first level: a section, or the gallery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    Section(Section),
    Gallery,
}

pub const ROOT: [Entry; 7] = [
    Entry::Section(Section::Picture),
    Entry::Section(Section::Exposure),
    Entry::Section(Section::Overlays),
    Entry::Section(Section::Recording),
    Entry::Gallery,
    Entry::Section(Section::Setup),
    Entry::Section(Section::System),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Brightness,
    Contrast,
    Saturation,
    Sharpness,
    ResetPicture,
    AutoExposure,
    ExposureBias,
    ExposureLock,
    WhiteBalanceLock,
    Cross,
    AddLine,
    ClearLines,
    LevelHdmi,
    LevelUsb,
    LevelMedia,
    RecordSize,
    Mirror,
    PowerLine,
    Aspect,
    Hdmi,
    Priority,
    Language,
    Card,
    Reset,
    About,
}

/// What an item is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// a number edited on the slider: its control
    Number(&'static str),
    /// one of a list of values: two toggle in place, more open the list
    Value,
    /// an action, or a level of its own
    Action,
}

impl Section {
    pub fn items(self) -> &'static [Item] {
        use Item::*;
        match self {
            Section::Picture => &[Brightness, Contrast, Saturation, Sharpness, ResetPicture],
            Section::Exposure => &[AutoExposure, ExposureBias, ExposureLock, WhiteBalanceLock],
            Section::Overlays => &[Cross, AddLine, ClearLines, LevelHdmi, LevelUsb, LevelMedia],
            Section::Recording => &[RecordSize],
            Section::Setup => &[Mirror, PowerLine, Aspect, Hdmi, Priority],
            Section::System => &[Language, Card, Reset, About],
        }
    }

    pub fn of(item: Item) -> Section {
        [Section::Picture, Section::Exposure, Section::Overlays, Section::Recording, Section::Setup, Section::System]
            .into_iter()
            .find(|s| s.items().contains(&item))
            .expect("every item is in a section")
    }

    pub fn name(self) -> T {
        match self {
            Section::Picture => T("Картинка", "Picture"),
            Section::Exposure => T("Экспозиция", "Exposure"),
            Section::Overlays => T("Наложения", "Overlays"),
            Section::Recording => T("Запись", "Recording"),
            Section::Setup => T("Установка", "Setup"),
            Section::System => T("Система", "System"),
        }
    }

    pub fn help(self) -> T {
        match self {
            Section::Picture => T("Яркость, контраст, насыщенность и резкость изображения.", "The picture's brightness, contrast, saturation and sharpness."),
            Section::Exposure => T("Автоматика яркости кадра и баланс белого: доверить камере, сдвинуть или зафиксировать.", "The automatic brightness of the frame and the white balance: leave them to the camera, shift them or hold them."),
            Section::Overlays => T("Перекрестие и линии поверх картинки, и на каких выходах их показывать.", "The crosshair and lines over the picture, and on which outputs to show them."),
            Section::Recording => T("Качество видео, которое камера пишет на SD-карту.", "The quality of the video the camera writes to the SD card."),
            Section::Setup => T("То, что настраивают один раз под рабочее место: зеркало, электросеть, монитор.", "What is set once for the workplace: the mirror, the mains, the monitor."),
            Section::System => T("Язык, SD-карта, сброс настроек и сведения о камере.", "The language, the SD card, the settings reset and what the camera is."),
        }
    }
}

impl Item {
    /// The ◐ key's round of the picture's numbers.
    pub const PICTURE: [Item; 4] = [Item::Brightness, Item::Contrast, Item::Saturation, Item::Sharpness];

    pub fn kind(self) -> Kind {
        use Item::*;
        match self {
            Brightness => Kind::Number("brightness"),
            Contrast => Kind::Number("contrast"),
            Saturation => Kind::Number("saturation"),
            Sharpness => Kind::Number("sharpness"),
            ExposureBias => Kind::Number("exposure-bias"),
            AutoExposure | ExposureLock | WhiteBalanceLock | Cross | LevelHdmi | LevelUsb | LevelMedia | RecordSize | Mirror | PowerLine | Aspect | Hdmi | Priority | Language => Kind::Value,
            ResetPicture | AddLine | ClearLines | Card | Reset | About => Kind::Action,
        }
    }

    /// The camera control a value item is one of (the mirror is two, `mirror` and `flip`; the crosshair and the
    /// language are the menu's own).
    pub fn control(self) -> Option<&'static str> {
        use Item::*;
        Some(match self {
            Brightness | Contrast | Saturation | Sharpness | ExposureBias => match self.kind() {
                Kind::Number(c) => c,
                _ => unreachable!("a number"),
            },
            AutoExposure => "auto-exposure",
            ExposureLock => "exposure-lock",
            WhiteBalanceLock => "white-balance-lock",
            LevelHdmi => "level-hdmi",
            LevelUsb => "level-usb",
            LevelMedia => "level-media",
            RecordSize => "record-size",
            PowerLine => "power-line",
            Aspect => "aspect",
            Hdmi => "hdmi",
            Priority => "precedence",
            _ => return None,
        })
    }

    pub fn name(self) -> T {
        use Item::*;
        match self {
            Brightness => T("Яркость", "Brightness"),
            Contrast => T("Контраст", "Contrast"),
            Saturation => T("Насыщенность", "Saturation"),
            Sharpness => T("Резкость", "Sharpness"),
            ResetPicture => T("Сбросить раздел", "Reset the section"),
            AutoExposure => T("Автоэкспозиция", "Auto exposure"),
            ExposureBias => T("Сдвиг экспозиции", "Exposure shift"),
            ExposureLock => T("Замок экспозиции", "Exposure lock"),
            WhiteBalanceLock => T("Баланс белого", "White balance"),
            Cross => T("Перекрестие", "Crosshair"),
            AddLine => T("Добавить линию", "Add a line"),
            ClearLines => T("Убрать все линии", "Remove all lines"),
            LevelHdmi => T("Показ на HDMI", "Shown on HDMI"),
            LevelUsb => T("Показ на USB", "Shown on USB"),
            LevelMedia => T("В записи и снимке", "In recordings and photos"),
            RecordSize => T("Качество видео", "Video quality"),
            Mirror => T("Зеркало", "Mirror"),
            PowerLine => T("Частота сети", "Mains frequency"),
            Aspect => T("Формат экрана", "Screen format"),
            Hdmi => T("Режим HDMI", "HDMI mode"),
            Priority => T("Приоритет", "Priority"),
            Language => T("Язык", "Language"),
            Card => T("SD-карта", "SD card"),
            Reset => T("Сброс настроек", "Settings reset"),
            About => T("О камере", "About the camera"),
        }
    }

    pub fn help(self) -> T {
        use Item::*;
        match self {
            Brightness => T("Общая яркость изображения. Заводское значение — 50.", "The picture's overall brightness. The factory value is 50."),
            Contrast => T("Разница между светлым и тёмным. Заводское значение — 50.", "The difference between light and dark. The factory value is 50."),
            Saturation => T("Сила цвета: от серой картинки до очень яркой. Заводское значение — 50.", "The strength of colour: from a grey picture to a very bright one. The factory value is 50."),
            Sharpness => T("Подчёркивание контуров. Большие значения дают светлую кайму вокруг дорожек и шум на тёмном.", "How much edges are stressed. High values give a light fringe around the traces and noise in the dark."),
            ResetPicture => T("Вернуть яркость, контраст, насыщенность и резкость к заводским значениям.", "Brightness, contrast, saturation and sharpness back to their factory values."),
            AutoExposure => T("Вкл — камера сама подбирает выдержку и усиление под то, что в кадре. Выкл — они остаются такими, какими были в момент выключения.", "On — the camera picks the exposure time and gain for what is in the frame. Off — they stay as they were at the moment it was switched off."),
            ExposureBias => T("Делает картинку светлее или темнее, чем выбрала бы автоматика. Плюс — когда тёмная плата выходит слишком тёмной, минус — когда блики на припое выгорают в белое. Работает только при включённой автоэкспозиции.", "Makes the picture lighter or darker than the automatics would. Plus when a dark board comes out too dark, minus when the glints on solder burn out to white. Works only with auto exposure on."),
            ExposureLock => T("Фиксирует текущую яркость кадра: она перестаёт плавать, когда в кадр входит паяльник или пинцет. То же делает кнопка AE на пульте. После выключения камеры замок снят.", "Holds the frame's brightness as it is now: it stops drifting when an iron or tweezers come into the frame. The AE key on the remote does the same. After a power-off the lock is gone."),
            WhiteBalanceLock => T("Авто — камера сама убирает цветной оттенок света. Замок фиксирует текущий баланс, чтобы цвета не уплывали, когда в кадре много одного цвета — например, зелёной маски платы. То же делает кнопка AWB на пульте.", "Auto — the camera takes the light's tint away by itself. The lock holds the balance as it is now, so the colours do not drift when much of the frame is one colour — the board's green mask, say. The AWB key on the remote does the same."),
            Cross => T("Метка в центре кадра. То же делает кнопка + на пульте.", "A mark in the centre of the frame. The + key on the remote does the same."),
            AddLine => T("Прямая через весь экран — выровнять плату или сверить положение деталей. Линия привязана к экрану и за платой не двигается. Линий может быть сколько угодно; после выключения камеры их нет.", "A straight line across the whole screen — to align the board or check where parts sit. The line belongs to the screen and does not follow the board. There may be any number of lines; after a power-off they are gone."),
            ClearLines => T("Убирает все поставленные линии. Перекрестие остаётся.", "Removes every line put. The crosshair stays."),
            LevelHdmi => T("Что из нарисованного камерой видно на мониторе: только картинка, картинка с перекрестием и линиями, ещё и плашки состояния, или всё вместе с меню.", "What of the camera's drawing the monitor shows: the picture alone, the picture with the crosshair and lines, the state chips as well, or everything with the menu."),
            LevelUsb => T("То же для картинки, которая идёт в компьютер по USB.", "The same for the picture that goes to the computer over USB."),
            LevelMedia => T("Что попадает в видео и снимки на SD-карте. Заводское значение — только картинка: линии и меню в файл не пишутся.", "What goes into the videos and photos on the SD card. The factory value is the picture alone: lines and the menu are not written into the file."),
            RecordSize => T("4K — 3840×2160, файлы крупнее. 1080p — 1920×1080, на карту помещается больше.", "4K — 3840×2160, larger files. 1080p — 1920×1080, more fits on the card."),
            Mirror => T("Переворачивает картинку, если камера стоит над столом повёрнутой. Подберите так, чтобы плата на экране двигалась туда же, куда её двигает рука. То же делают кнопки ⇔ и ⇕ на пульте.", "Turns the picture over when the camera stands turned above the desk. Pick it so the board on the screen moves where the hand moves it. The ⇔ and ⇕ keys on the remote do the same."),
            PowerLine => T("Частота электросети, от которой питается освещение: 50 Гц в Европе и России, 60 Гц в Америке. При неверной по картинке идут тёмные полосы.", "The frequency of the mains the lights run on: 50 Hz in Europe and Russia, 60 Hz in America. A wrong one sends dark bands over the picture."),
            Aspect => T("Соотношение сторон монитора. Если круглое на экране выглядит овалом — выбран не тот формат.", "The monitor's aspect ratio. If something round looks oval on the screen, the format is wrong."),
            Hdmi => T("Разрешение и частота, с которыми камера выдаёт картинку на монитор. Авто — лучшее из того, что монитор сообщил о себе.", "The resolution and rate the camera gives the monitor. Auto — the best of what the monitor said it takes."),
            Priority => T("Какой выход важнее. Когда камере не хватает сил на оба сразу, этот остаётся в полном качестве, а у второго качество снижается.", "Which output matters more. When the camera cannot serve both in full, this one keeps its full quality and the other's is lowered."),
            Language => T("Язык меню.", "The menu's language."),
            Card => T("Свободное место на карте и её форматирование.", "The card's free space and its formatting."),
            Reset => T("Возвращает все настройки к заводским. Файлы на SD-карте не трогает.", "Every setting back to the factory's. The files on the SD card stay."),
            About => T("Версия прошивки и текущие режимы выходов.", "The firmware's version and the outputs' modes now."),
        }
    }
}
