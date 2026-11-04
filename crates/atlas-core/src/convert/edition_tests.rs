use super::*;
use atlas_model::edition::{Edition, Feature, Limit, BUILD};

fn source(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    image::RgbImage::from_pixel(2, 2, image::Rgb([32, 64, 128]))
        .save(&path)
        .unwrap();
    path
}

#[test]
fn edition_single_conversion_remains_available() {
    let dir = tempfile::tempdir().unwrap();
    let path = source(dir.path(), "single.png");
    let original = std::fs::read(&path).unwrap();
    let plan = Plan::build_native(
        std::slice::from_ref(&path),
        "bmp",
        Settings::default(),
        Tools::default(),
    )
    .unwrap();
    let report = execute(&plan, &Arc::new(AtomicBool::new(false)), |_, _, _| {});
    assert!(report.errors().is_empty(), "{:?}", report.errors());
    assert_eq!(report.outputs().len(), 1);
    assert_eq!(std::fs::read(&path).unwrap(), original);
}

#[test]
fn edition_builders_reject_batches_only_in_free() {
    let dir = tempfile::tempdir().unwrap();
    let paths = [source(dir.path(), "one.png"), source(dir.path(), "two.png")];
    for native in [false, true] {
        let result = if native {
            Plan::build_native(&paths, "bmp", Settings::default(), Tools::default())
        } else {
            Plan::build(&paths, "bmp", Settings::default(), Tools::default())
        };
        match BUILD {
            Edition::Free => assert_eq!(
                result.unwrap_err().downcast_ref::<Limit>(),
                Some(&Limit {
                    feature: Feature::BatchConversion,
                    maximum: 1
                }),
            ),
            Edition::Full => assert_eq!(result.unwrap().commands.len(), 2),
        }
    }
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
}

#[test]
fn edition_direct_batch_plan_cannot_bypass_the_executor() {
    let dir = tempfile::tempdir().unwrap();
    let paths = [source(dir.path(), "one.png"), source(dir.path(), "two.png")];
    let originals: Vec<_> = paths
        .iter()
        .map(|path| std::fs::read(path).unwrap())
        .collect();
    let target = FORMATS
        .iter()
        .find(|format| format.extension == "bmp")
        .unwrap();
    let plan = Plan {
        commands: paths
            .iter()
            .map(|path| Command {
                source: path.clone(),
                target,
                backend: Backend::Image,
            })
            .collect(),
        settings: Settings::default(),
        tools: Tools::default(),
    };
    let completed = AtomicUsize::new(0);
    let report = execute(&plan, &Arc::new(AtomicBool::new(false)), |_, total, _| {
        assert_eq!(total, 2);
        completed.fetch_add(1, Ordering::Relaxed);
    });
    assert_eq!(completed.load(Ordering::Relaxed), 2);
    assert_eq!(report.outcomes.len(), 2);
    if BUILD == Edition::Free {
        assert!(report.outputs().is_empty());
        assert!(report
            .errors()
            .iter()
            .all(|error| error.contains("Atlas Free")));
        assert_eq!(report.errors().len(), 2);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    } else {
        assert!(report.errors().is_empty(), "{:?}", report.errors());
        assert_eq!(report.outputs().len(), 2);
    }
    for (path, original) in paths.iter().zip(originals) {
        assert_eq!(std::fs::read(path).unwrap(), original);
    }
}
