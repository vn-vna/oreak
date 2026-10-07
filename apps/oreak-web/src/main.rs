#[cfg(any(target_arch = "wasm32", test))]
mod api;
#[cfg(any(target_arch = "wasm32", test))]
mod image_geometry;
#[cfg(any(target_arch = "wasm32", test))]
mod model;
#[cfg(target_arch = "wasm32")]
mod rpc;
#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(target_arch = "wasm32")]
fn main() {
    yew::Renderer::<web::App>::new().render();
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    println!("oreak-web is a browser application; build it for wasm32-unknown-unknown");
}
