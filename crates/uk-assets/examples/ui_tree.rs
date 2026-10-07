//! The uGUI tree under every Canvas in a level: active flags, RectTransform fields, Canvas/CanvasGroup
//! data and the Graphic components (class, color, sprite/text). `--all` includes inactive subtrees.
//! cargo run --release -p uk-assets --example ui_tree -- level0-1 [path-filter] [--all]
use uk_assets::{db::AssetDb, scenedef};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let all = args.iter().any(|a| a == "--all");
    let pos: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    let level = pos.first().map(|s| s.as_str()).unwrap_or("level0-1").to_string();
    let filter = pos.get(1).map(|s| s.to_string()).unwrap_or_default();
    let install = uk_assets::find_install().expect("install");
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"));
    let def = scenedef::load_scene(&mut db, &path).unwrap();
    let scene = db.file(&def.scene_file).unwrap();
    let is_canvas = |n: u32| def.ui_natives.iter().any(|u| u.node == n && u.class_id == 223);
    let roots: Vec<u32> = (0..def.nodes.len() as u32)
        .filter(|&n| is_canvas(n) && !def.nodes[n as usize].parent.is_some_and(|p| is_canvas_anc(&def, p, &is_canvas)))
        .filter(|&n| def.path(n).contains(&filter))
        .collect();
    for r in roots {
        let mut stack = vec![(r, 0usize, true)];
        while let Some((n, d, act)) = stack.pop() {
            let nd = &def.nodes[n as usize];
            let act = act && nd.active_self;
            if !act && !all { continue }
            let rect = nd.rect.map(|r| format!("amin{:?} amax{:?} pos{:?} size{:?} piv{:?}", r.anchor_min, r.anchor_max, r.anchored_pos, r.size_delta, r.pivot)).unwrap_or_default();
            println!("{}{}{} [{}] L{} s{:?} {}", "  ".repeat(d), if act { "" } else { "(off) " }, nd.name, n, nd.layer, nd.local_scale.to_array(), rect);
            for u in def.ui_natives.iter().filter(|u| u.node == n) {
                let v = &u.data;
                if u.class_id == 223 {
                    println!("{}  = Canvas en{} mode{} order{} override{} pp{} cam{:?} plane{}", "  ".repeat(d), v.get("m_Enabled").bool(), v.get("m_RenderMode").i64(), v.get("m_SortingOrder").i64(), v.get("m_OverrideSorting").bool(), v.get("m_PixelPerfect").bool(), v.get("m_Camera").pptr(), v.get("m_PlaneDistance").f32());
                } else {
                    println!("{}  = CanvasGroup en{} alpha{} ignoreParent{}", "  ".repeat(d), v.get("m_Enabled").bool(), v.get("m_Alpha").f32(), v.get("m_IgnoreParentGroups").bool());
                }
            }
            for (_, s) in def.scripts_on(n) {
                let file = match &s.file { Some(f) => db.file(f).unwrap(), None => scene.clone() };
                let c = s.data.get("m_Color");
                let col = format!("({:.3},{:.3},{:.3},{:.3})", c.get("r").f32(), c.get("g").f32(), c.get("b").f32(), c.get("a").f32());
                let extra = match s.class.as_str() {
                    "Image" => {
                        let sp = db.read_pptr(&file, s.data.get("m_Sprite").pptr()).ok().flatten().map(|(_, _, v)| v.get("m_Name").str().to_string()).unwrap_or("-".into());
                        format!("{col} sprite {sp} type{} fill{} amt{} origin{} cw{} preserve{}", s.data.get("m_Type").i64(), s.data.get("m_FillMethod").i64(), s.data.get("m_FillAmount").f32(), s.data.get("m_FillOrigin").i64(), s.data.get("m_FillClockwise").bool(), s.data.get("m_PreserveAspect").bool())
                    }
                    "RawImage" => format!("{col}"),
                    "Text" => format!("{col} {:?} size{}", s.data.get("m_Text").str(), s.data.get("m_FontData").get("m_FontSize").i64()),
                    "TextMeshProUGUI" => format!("{col} {:?} size{}", s.data.get("m_text").str(), s.data.get("m_fontSize").f32()),
                    _ => String::new(),
                };
                println!("{}  - {}{} {}", "  ".repeat(d), if s.enabled { "" } else { "(disabled) " }, s.class, extra);
            }
            for &c in nd.children.iter().rev() {
                stack.push((c, d + 1, act));
            }
        }
    }
}

fn is_canvas_anc(def: &scenedef::SceneDef, mut n: u32, f: &dyn Fn(u32) -> bool) -> bool {
    loop {
        if f(n) { return true }
        match def.nodes[n as usize].parent { Some(p) => n = p, None => return false }
    }
}
