use librtmp2::buffer::Buffer;
use librtmp2::ertmp::connect_amf::{
    read_four_cc_list_amf, read_video_fourcc_info_map_amf, write_four_cc_list_amf,
    write_video_fourcc_info_map_amf,
};
use librtmp2::ertmp::multitrack_media::foreach_track;
use librtmp2::types::{FOUR_CC_INFO_CAN_FORWARD, FourCcList, FrameType, VideoFourCcInfoMap};

#[test]
fn video_fourcc_info_map_round_trips_as_object_with_flags() {
    let mut offered = VideoFourCcInfoMap::default();
    offered.entries[0].cc[..4].copy_from_slice(b"vp09");
    offered.masks[0] = FOUR_CC_INFO_CAN_FORWARD;
    offered.count = 1;

    let mut wire = Buffer::new();
    write_video_fourcc_info_map_amf(&mut wire, &offered).unwrap();

    let mut parsed = VideoFourCcInfoMap::default();
    read_video_fourcc_info_map_amf(&mut wire, &mut parsed).unwrap();
    assert_eq!(parsed.count, 1);
    assert_eq!(&parsed.entries[0].cc[..4], b"vp09");
    assert_eq!(parsed.masks[0], FOUR_CC_INFO_CAN_FORWARD);
}

#[test]
fn fourcc_wildcard_survives_amf_round_trip() {
    let mut offered = FourCcList::default();
    offered.entries[0].cc[0] = b'*';
    offered.count = 1;

    let mut wire = Buffer::new();
    write_four_cc_list_amf(&mut wire, &offered).unwrap();

    let mut parsed = FourCcList::default();
    read_four_cc_list_amf(&mut wire, &mut parsed).unwrap();
    assert_eq!(parsed.count, 1);
    assert_eq!(parsed.entries[0].cc[0], b'*');
}

#[test]
fn many_tracks_many_codecs_reports_each_track_codec() {
    let payload = [
        0x86, 0x20, b'a', b'v', b'c', b'1', 0, 0, 0, 1, 0xAA, b'h', b'v', b'c', b'1', 1, 0, 0, 1,
        0xBB,
    ];
    let mut seen = Vec::new();
    assert!(foreach_track(FrameType::Video, &payload, |track| {
        seen.push((track.track_id, track.fourcc, track.payload.to_vec()));
    }));
    assert_eq!(
        seen,
        vec![(0, *b"avc1", vec![0xAA]), (1, *b"hvc1", vec![0xBB])]
    );
}
