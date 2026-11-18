use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use koral::graphics::{Graphics, MAX_IMAGE, Outcome, Request};

fn apc(keys: &str, payload: &[u8]) -> Vec<u8> {
    format!("G{keys};{}", STANDARD.encode(payload)).into_bytes()
}

fn graphics() -> Graphics {
    let mut graphics = Graphics::default();
    graphics.set_cell(10, 20);
    graphics
}

fn send(graphics: &mut Graphics, keys: &str, payload: &[u8]) -> Outcome {
    graphics.handle(&apc(keys, payload))
}

fn png_bytes(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(&vec![200; (width * height * 3) as usize]).unwrap();
    writer.finish().unwrap();
    bytes
}

/// A real 1x1 PNG whose IHDR lies about its size, like a crafted file piped into the terminal.
fn png_declaring(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = png_bytes(1, 1);
    bytes[16..20].copy_from_slice(&width.to_be_bytes());
    bytes[20..24].copy_from_slice(&height.to_be_bytes());
    let mut crc = flate2::Crc::new();
    crc.update(&bytes[12..29]); // the IHDR type and data
    bytes[29..33].copy_from_slice(&crc.sum().to_be_bytes());
    bytes
}

fn deflate(bytes: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

#[test]
fn raw_rgba_is_stored_under_its_id() {
    let mut g = graphics();
    send(&mut g, "a=t,f=32,s=1,v=2,i=7", &[1, 2, 3, 4, 5, 6, 7, 8]);
    let image = g.image(7).unwrap();
    assert_eq!((image.width, image.height), (1, 2));
    assert_eq!(image.rgba, vec![1, 2, 3, 4, 5, 6, 7, 8]);
}

#[test]
fn rgb_gains_an_opaque_alpha() {
    let mut g = graphics();
    send(&mut g, "a=t,f=24,s=1,v=1,i=1", &[9, 8, 7]);
    assert_eq!(g.image(1).unwrap().rgba, vec![9, 8, 7, 255]);
}

#[test]
fn png_is_decoded_without_being_told_its_size() {
    let mut g = graphics();
    send(&mut g, "a=t,f=100,i=1", &png_bytes(3, 2));
    let image = g.image(1).unwrap();
    assert_eq!((image.width, image.height), (3, 2));
    assert_eq!(&image.rgba[..4], &[200, 200, 200, 255]);
}

#[test]
fn a_png_header_declaring_a_huge_image_is_refused_before_its_buffer_is_allocated() {
    // 20000x20000 is 1.2 GB of RGB, far past MAX_IMAGE, and the header alone says so.
    let mut g = graphics();
    let reply = send(&mut g, "a=t,f=100,i=1", &png_declaring(20_000, 20_000)).reply.unwrap();
    assert!(reply.contains("EFBIG"), "{reply:?}");
    assert!(g.image(1).is_none());
}

#[test]
fn zlib_payloads_are_inflated() {
    let mut g = graphics();
    send(&mut g, "a=t,f=24,s=1,v=1,o=z,i=1", &deflate(&[9, 8, 7]));
    assert_eq!(g.image(1).unwrap().rgba, vec![9, 8, 7, 255]);
}

#[test]
fn chunks_join_into_one_image_and_keep_the_first_keys() {
    let mut g = graphics();
    let first = g.handle(&apc("a=t,f=32,s=1,v=1,i=4,m=1", &[1, 2, 3]));
    assert_eq!(first, Outcome::default());
    send(&mut g, "m=0", &[4]);
    assert_eq!(g.image(4).unwrap().rgba, vec![1, 2, 3, 4]);
}

#[test]
fn a_transmission_with_an_id_is_acknowledged() {
    let mut g = graphics();
    let outcome = send(&mut g, "a=t,f=24,s=1,v=1,i=5", &[1, 2, 3]);
    assert_eq!(outcome.reply.as_deref(), Some("\x1b_Gi=5;OK\x1b\\"));
}

#[test]
fn quiet_one_hides_success_but_not_errors() {
    let mut g = graphics();
    assert_eq!(send(&mut g, "a=t,f=24,s=1,v=1,i=5,q=1", &[1, 2, 3]).reply, None);
    let reply = send(&mut g, "a=t,f=24,s=9,v=9,i=5,q=1", &[1, 2, 3]).reply.unwrap();
    assert!(reply.starts_with("\x1b_Gi=5;E"), "{reply:?}");
}

#[test]
fn quiet_two_hides_errors_too() {
    let mut g = graphics();
    assert_eq!(send(&mut g, "a=t,f=24,s=9,v=9,i=5,q=2", &[1, 2, 3]).reply, None);
}

#[test]
fn a_query_answers_without_storing() {
    let mut g = graphics();
    let outcome = send(&mut g, "a=q,f=24,s=1,v=1,i=31", &[1, 2, 3]);
    assert_eq!(outcome.reply.as_deref(), Some("\x1b_Gi=31;OK\x1b\\"));
    assert!(g.image(31).is_none());
}

#[test]
fn file_transmission_is_refused_so_a_program_cannot_read_local_files() {
    for medium in ["f", "t", "s"] {
        let mut g = graphics();
        let keys = format!("a=t,f=100,t={medium},i=1");
        let reply = send(&mut g, &keys, b"/etc/passwd").reply.unwrap();
        assert!(reply.contains("ENOTSUPPORTED"), "{reply:?}");
        assert!(g.image(1).is_none());
    }
}

/// The command icat sends for `--transfer-mode=memory`: the payload is a shared memory name.
#[test]
fn a_shared_memory_transmission_is_refused_with_its_error() {
    let mut g = graphics();
    let name = STANDARD.encode(b"/icat-BWVCZONVJ6XL2");
    let reply = send(&mut g, "a=T,f=100,t=s,s=240,v=120,S=401,i=7", name.as_bytes()).reply.unwrap();
    assert!(reply.contains("ENOTSUPPORTED"), "{reply:?}");
    assert!(g.image(7).is_none());
}

/// A client probes the mediums with a query before it picks one; every unsupported probe must
/// come back so it can fall back to direct transmission.
#[test]
fn a_query_over_an_unsupported_medium_reports_it() {
    for medium in ["f", "t", "s"] {
        let mut g = graphics();
        let keys = format!("a=q,f=24,s=1,v=1,t={medium},i=3");
        let reply = send(&mut g, &keys, b"/dev/shm/kitty-tty-graphics-protocol-1").reply.unwrap();
        assert!(reply.contains("ENOTSUPPORTED"), "{reply:?}");
    }
}

#[test]
fn an_action_the_terminal_does_not_implement_is_answered() {
    let mut g = graphics();
    let reply = send(&mut g, "a=f,f=24,s=1,v=1,i=9", &[0; 3]).reply.unwrap();
    assert!(reply.contains("EINVAL"), "{reply:?}");
}

#[test]
fn an_error_without_an_id_stays_silent() {
    // A reply is routed by image id, and icat also asks for silence with q=2: a client that sends
    // neither has nothing to read, so the refusal can only reach it when it sends an id.
    let mut g = graphics();
    assert_eq!(send(&mut g, "a=T,f=100,t=s,s=1,v=1", b"/dev/shm/x").reply, None);
}

#[test]
fn a_size_past_the_limit_is_refused() {
    let side = (MAX_IMAGE as f64).sqrt() as u32;
    let mut g = graphics();
    let keys = format!("a=t,f=32,s={side},v={side},i=1");
    let reply = send(&mut g, &keys, &[0; 4]).reply.unwrap();
    assert!(reply.contains("EFBIG") || reply.contains("EINVAL"), "{reply:?}");
    assert!(g.image(1).is_none());
}

#[test]
fn old_images_leave_when_the_store_is_full() {
    let mut g = graphics();
    let side = 4000usize;
    let pixels = vec![0; side * side * 4];
    for id in 1..=6 {
        let keys = format!("a=t,f=32,s={side},v={side},i={id},q=2");
        send(&mut g, &keys, &pixels);
    }
    assert!(g.image(1).is_none());
    assert!(g.image(6).is_some());
}

/// 4000x4000 RGBA is 64 MB, a fifth of the store: six of them do not fit.
fn transmit_a_fifth(graphics: &mut Graphics, id: u32) {
    send(graphics, &format!("a=t,f=32,s=4000,v=4000,i={id},q=2"), &vec![0; 4000 * 4000 * 4]);
}

fn shown(image: u32) -> Request {
    Request { image, number: 0, columns: 1, rows: 1, keep_cursor: true, crop: [0, 0, 1, 1], letterbox: false, z: 0 }
}

#[test]
fn an_image_on_screen_survives_a_burst_of_previews() {
    let mut g = graphics();
    transmit_a_fifth(&mut g, 1);
    let anchor = g.place(shown(1), &[]).expect("a placement");
    for id in 2..=6 {
        transmit_a_fifth(&mut g, id);
    }
    assert!(g.image(1).is_some(), "the image a placement shows was evicted");
    assert!(g.placement(anchor).is_some());
    assert!(g.image(2).is_none(), "the oldest preview stayed");
    assert!(g.image(6).is_some());
}

#[test]
fn a_soft_deleted_image_leaves_before_one_on_screen() {
    let mut g = graphics();
    transmit_a_fifth(&mut g, 1);
    let anchor = g.place(shown(1), &[]).expect("a placement");
    transmit_a_fifth(&mut g, 2);
    g.place(shown(2), &[]).expect("a placement");
    send(&mut g, "a=d,d=i,i=2", &[]);
    for id in 3..=6 {
        transmit_a_fifth(&mut g, id);
    }
    assert!(g.image(1).is_some());
    assert!(g.placement(anchor).is_some());
    assert!(g.image(2).is_none(), "the deleted image kept its quota");
}

#[test]
fn used_matches_the_images_that_remain() {
    let mut g = graphics();
    for id in 1..=6 {
        transmit_a_fifth(&mut g, id);
    }
    let total: usize = (1..=6).filter_map(|id| g.image(id)).map(|image| image.rgba.len()).sum();
    assert_eq!(g.used(), total);
    assert!(total <= 5 * MAX_IMAGE, "{total} bytes kept");
}

#[test]
fn transmit_and_place_asks_for_a_placement_in_cells() {
    let mut g = graphics();
    let outcome = send(&mut g, "a=T,f=24,s=25,v=30,i=2,q=2", &vec![0; 25 * 30 * 3]);
    let request = outcome.place.unwrap();
    assert_eq!(
        request,
        Request { image: 2, number: 0, columns: 3, rows: 2, keep_cursor: false, crop: [0, 0, 25, 30], letterbox: false, z: 0 }
    );
}

#[test]
fn explicit_cells_and_the_cursor_key_win() {
    let mut g = graphics();
    send(&mut g, "a=t,f=24,s=1,v=1,i=2", &[0; 3]);
    let request = send(&mut g, "a=p,i=2,p=9,c=40,r=12,C=1", &[]).place.unwrap();
    assert_eq!(
        request,
        Request { image: 2, number: 9, columns: 40, rows: 12, keep_cursor: true, crop: [0, 0, 1, 1], letterbox: true, z: 0 }
    );
}

#[test]
fn a_placement_keeps_the_part_of_the_image_it_asked_for() {
    let mut g = graphics();
    send(&mut g, "a=t,f=24,s=4,v=4,i=2", &[0; 4 * 4 * 3]);
    let request = send(&mut g, "a=p,i=2,x=1,y=2,w=2,h=1,c=1,r=1,C=1", &[]).place.unwrap();
    assert_eq!(request.crop, [1, 2, 2, 1]);
}

#[test]
fn a_crop_ends_at_the_edge_of_the_image() {
    let mut g = graphics();
    send(&mut g, "a=t,f=24,s=4,v=4,i=2", &[0; 4 * 4 * 3]);
    assert_eq!(send(&mut g, "a=p,i=2,x=3,y=1", &[]).place.unwrap().crop, [3, 1, 1, 3]);
    assert_eq!(send(&mut g, "a=p,i=2,x=9,y=9,w=9,h=9", &[]).place.unwrap().crop, [3, 3, 1, 1]);
}

#[test]
fn a_crop_without_cells_takes_the_size_of_the_crop() {
    let mut g = graphics();
    send(&mut g, "a=t,f=24,s=30,v=30,i=2", &vec![0; 30 * 30 * 3]);
    let request = send(&mut g, "a=p,i=2,w=10,h=30", &[]).place.unwrap();
    assert_eq!((request.columns, request.rows), (1, 2));
}

#[test]
fn placing_a_missing_image_reports_enoent() {
    let mut g = graphics();
    let outcome = send(&mut g, "a=p,i=8", &[]);
    assert!(outcome.reply.unwrap().contains("ENOENT"));
    assert_eq!(outcome.place, None);
}

#[test]
fn a_transmission_without_an_id_still_places() {
    let mut g = graphics();
    let request = send(&mut g, "a=T,f=24,s=1,v=1", &[1, 2, 3]).place.unwrap();
    assert!(g.image(request.image).is_some());
}

#[test]
fn a_placement_with_a_number_replaces_the_old_one() {
    let mut g = graphics();
    send(&mut g, "a=t,f=24,s=1,v=1,i=1", &[0; 3]);
    let request =
        Request { image: 1, number: 3, columns: 1, rows: 1, keep_cursor: false, crop: [0, 0, 1, 1], letterbox: false, z: 0 };
    let first = g.place(request, &[]).expect("a placement");
    let second = g.place(request, &[]).expect("a placement");
    assert_ne!(first, second);
    assert!(g.placement(first).is_none());
    assert!(g.placement(second).is_some());
}

#[test]
fn placing_again_on_the_same_cell_reuses_its_anchor() {
    let mut g = graphics();
    send(&mut g, "a=t,f=24,s=1,v=1,i=1", &[0; 3]);
    let request =
        Request { image: 1, number: 0, columns: 1, rows: 1, keep_cursor: true, crop: [0, 0, 1, 1], letterbox: false, z: 0 };
    let first = g.place(request, &[]).expect("a placement");
    assert_eq!(g.place(request, &[first]), Some(first));
}

#[test]
fn deleting_everything_removes_placements_and_keeps_images() {
    let mut g = graphics();
    send(&mut g, "a=t,f=24,s=1,v=1,i=1", &[0; 3]);
    let anchor = g
        .place(Request { image: 1, number: 0, columns: 1, rows: 1, keep_cursor: true, crop: [0, 0, 1, 1], letterbox: false, z: 0 }, &[])
        .expect("a placement");
    send(&mut g, "a=d,d=a", &[]);
    assert!(g.placement(anchor).is_none());
    assert!(g.image(1).is_some());
}

#[test]
fn capital_delete_frees_the_images_too() {
    let mut g = graphics();
    send(&mut g, "a=t,f=24,s=1,v=1,i=1", &[0; 3]);
    send(&mut g, "a=d,d=A", &[]);
    assert!(g.image(1).is_none());
}

#[test]
fn deleting_by_id_leaves_the_others() {
    let mut g = graphics();
    send(&mut g, "a=t,f=24,s=1,v=1,i=1", &[0; 3]);
    send(&mut g, "a=t,f=24,s=1,v=1,i=2", &[0; 3]);
    let one = g.place(Request { image: 1, number: 0, columns: 1, rows: 1, keep_cursor: true, crop: [0, 0, 1, 1], letterbox: false, z: 0 }, &[])
        .expect("a placement");
    let two = g.place(Request { image: 2, number: 0, columns: 1, rows: 1, keep_cursor: true, crop: [0, 0, 1, 1], letterbox: false, z: 0 }, &[])
        .expect("a placement");
    send(&mut g, "a=d,d=I,i=1", &[]);
    assert!(g.placement(one).is_none() && g.image(1).is_none());
    assert!(g.placement(two).is_some() && g.image(2).is_some());
}

#[test]
fn malformed_commands_change_nothing() {
    let mut g = graphics();
    for body in [&b"G"[..], b"Gnonsense", b"G=;", b"Ga=t,s=abc;!!!", b"G\xff\xfe"] {
        let _ = g.handle(body);
    }
    assert!(g.image(0).is_none());
}

#[test]
fn a_new_transmission_of_an_id_counts_as_a_new_picture() {
    let mut g = graphics();
    send(&mut g, "a=t,f=24,s=1,v=1,i=1", &[0; 3]);
    let first = g.image(1).unwrap().generation;
    send(&mut g, "a=t,f=24,s=1,v=1,i=1", &[1; 3]);
    assert_ne!(g.image(1).unwrap().generation, first);
}

/// A thumbnail and the viewer show one transmission in two places.
#[test]
fn one_image_shows_in_two_places() {
    let mut g = graphics();
    send(&mut g, "a=t,f=24,s=1,v=1,i=1", &[0; 3]);
    let first = g.place(shown(1), &[]).expect("a placement");
    let second = g.place(shown(1), &[]).expect("a placement");
    assert_ne!(first, second);
    assert!(g.placement(first).is_some() && g.placement(second).is_some());
}

#[test]
fn a_placement_id_tells_two_placements_of_one_image_apart() {
    let mut g = graphics();
    send(&mut g, "a=t,f=24,s=1,v=1,i=1", &[0; 3]);
    let one = g.place(Request { number: 4, ..shown(1) }, &[]).expect("a placement");
    let two = g.place(Request { number: 5, ..shown(1) }, &[]).expect("a placement");
    assert!(g.placement(one).is_some() && g.placement(two).is_some());
}

/// The same placement id is the same placement: the second one replaces the first, as kitty says.
#[test]
fn the_same_placement_id_replaces_the_earlier_one() {
    let mut g = graphics();
    send(&mut g, "a=t,f=24,s=1,v=1,i=1", &[0; 3]);
    let one = g.place(Request { number: 4, ..shown(1) }, &[]).expect("a placement");
    let two = g.place(Request { number: 4, ..shown(1) }, &[]).expect("a placement");
    assert_eq!(g.placement(one), None);
    assert!(g.placement(two).is_some());
}

#[test]
fn a_transmit_then_a_place_shows_the_stored_image() {
    let mut g = graphics();
    send(&mut g, "a=t,f=24,s=1,v=1,i=1", &[0; 3]);
    let outcome = send(&mut g, "a=p,i=1,c=2,r=1,C=1", &[]);
    assert_eq!(outcome.place.expect("a placement").image, 1);
    assert!(g.image(1).is_some());
}

/// A smaller re-transmission cannot show through the rectangle the old size asked for.
#[test]
fn transmitting_a_different_size_drops_the_placements() {
    let mut g = graphics();
    send(&mut g, "a=t,f=24,s=2,v=2,i=1", &[0; 12]);
    let anchor = g.place(shown(1), &[]).expect("a placement");
    send(&mut g, "a=t,f=24,s=1,v=1,i=1", &[1; 3]);
    assert!(g.placement(anchor).is_none());
    assert!(g.image(1).is_some());
}

/// The frame contract: repainting the same id at the same size keeps the anchor on its cell.
#[test]
fn repainting_an_id_at_the_same_size_keeps_its_placement() {
    let mut g = graphics();
    send(&mut g, "a=t,f=24,s=1,v=1,i=1", &[0; 3]);
    let anchor = g.place(shown(1), &[]).expect("a placement");
    send(&mut g, "a=t,f=24,s=1,v=1,i=1", &[1; 3]);
    assert!(g.placement(anchor).is_some());
}

#[test]
fn a_placement_with_no_cells_is_refused() {
    let mut g = graphics();
    send(&mut g, "a=t,f=24,s=1,v=1,i=1", &[0; 3]);
    assert_eq!(g.place(Request { columns: 0, ..shown(1) }, &[]), None);
    assert_eq!(g.place(Request { rows: 0, ..shown(1) }, &[]), None);
    assert_eq!(g.tallest(), 0);
}

#[test]
fn a_delete_mode_this_terminal_cannot_resolve_answers() {
    let mut g = graphics();
    send(&mut g, "a=t,f=24,s=1,v=1,i=1", &[0; 3]);
    let anchor = g.place(shown(1), &[]).expect("a placement");
    let reply = send(&mut g, "a=d,d=c,i=1", &[]).reply.expect("an answer");
    assert!(reply.contains("ENOTSUPPORTED"), "{reply:?}");
    assert!(g.placement(anchor).is_some());
}

#[test]
fn a_delete_by_placement_id_leaves_the_other_placements() {
    let mut g = graphics();
    send(&mut g, "a=t,f=24,s=1,v=1,i=1", &[0; 3]);
    let one = g.place(Request { number: 4, ..shown(1) }, &[]).expect("a placement");
    let two = g.place(Request { number: 5, ..shown(1) }, &[]).expect("a placement");
    send(&mut g, "a=d,d=i,i=1,p=4", &[]);
    assert!(g.placement(one).is_none());
    assert!(g.placement(two).is_some());
}
