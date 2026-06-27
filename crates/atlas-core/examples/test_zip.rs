use std::path::Path;

fn main() {
    let p = Path::new("folder/");
    println!("p: {:?}", p);
    println!("parent: {:?}", p.parent());
    let s = p.to_string_lossy();
    let key = if s.is_empty() {
        String::new()
    } else {
        format!("{s}/")
    };
    println!("key: {:?}", key);
}
