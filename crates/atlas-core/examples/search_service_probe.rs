use atlas_core::search::{SearchBackend, SearchQuery, WindowsServiceBackend};

fn main() {
    let backend = WindowsServiceBackend::new();
    println!(
        "raw status: {:#?}",
        backend.request(&atlas_core::search::ServiceRequest::Status)
    );
    println!("status: {:#?}", backend.status());
    let mut query = SearchQuery::text("winsxs");
    query.limit = 10;
    let batch = backend.search(&query);
    println!(
        "generation: {}, complete: {}",
        batch.generation, batch.complete
    );
    for result in batch.results {
        println!("{}", result.path.display());
    }
}
