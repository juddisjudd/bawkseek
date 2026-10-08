use std::borrow::Cow;

use gpui_kit::assets::{Assets, icon_assets};
use gpui_kit::{AssetSource, Result, SharedString};

icon_assets!(
    AppIcons,
    [
        Activity,
        ArrowDownToLine,
        Ban,
        CircleAlert,
        CircleCheck,
        CircleX,
        Clock,
        Compass,
        Download,
        ExternalLink,
        FileMusic,
        FolderDown,
        FolderSearch,
        Gauge,
        Hash,
        Hourglass,
        ListFilter,
        Lock,
        LogOut,
        MessagesSquare,
        Pause,
        Play,
        RotateCw,
        Share2,
        SlidersHorizontal,
        Trash,
        Upload,
        UserMinus,
        UserPlus,
        Users,
        Wifi,
        WifiOff,
        X,
    ]
);

pub const MARK: &str = "brand/mark.svg";

pub const FONTS: [&[u8]; 4] = [
    include_bytes!("../assets/fonts/IBMPlexMono-Regular.ttf"),
    include_bytes!("../assets/fonts/IBMPlexMono-Medium.ttf"),
    include_bytes!("../assets/fonts/IBMPlexMono-SemiBold.ttf"),
    include_bytes!("../assets/fonts/IBMPlexMono-Bold.ttf"),
];

pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path == MARK {
            return Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/brand/mark.svg"
            ))));
        }
        if let Some(bytes) = AppIcons.load(path)? {
            return Ok(Some(bytes));
        }
        Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = Assets.list(path)?;
        paths.extend(AppIcons.list(path)?);
        if MARK.starts_with(path) {
            paths.push(MARK.into());
        }
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}
