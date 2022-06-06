use pathfinder_rasterize::Rasterizer;
use pathfinder_svg::SVGScene;
use usvg::{Options, Tree};

fn main() {
    let mut args = std::env::args();
    args.next().unwrap();

    let input = args.next().expect("input");
    let output = args.next().expect("output");

    let input_data = std::fs::read(&input).expect("read input");
    let tree = Tree::from_data(&input_data, &Options::default().to_ref()).unwrap();
    let scene = SVGScene::from_tree(&tree).scene;

    let image = Rasterizer::new().rasterize(scene, None);
    image.save(&output).unwrap();
}
