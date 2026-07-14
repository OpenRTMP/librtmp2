from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    file = Path(path)
    text = file.read_text()
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected exactly one match, found {count}")
    file.write_text(text.replace(old, new, 1))


replace_once(
    "src/client/mod.rs",
    "use crate::media::populate_av_frame;",
    "use crate::media::{is_on_metadata_payload, populate_av_frame};",
)

replace_once(
    "src/client/mod.rs",
    (
        "                        if let Some(ref cb) = self.on_frame_cb {\n"
        "                            self.frame_cb_scratch = data_payload;\n"
        "                            let frame = Frame {\n"
        "                                frame_type: FrameType::Script,\n"
        "                                timestamp: msg.timestamp,\n"
        "                                size: self.frame_cb_scratch.len() as u32,\n"
        "                                data: self.frame_cb_scratch.as_ptr(),\n"
        "                                is_metadata: 1,\n"
        "                                ..Default::default()\n"
        "                            };\n"
        "                            cb(&frame);\n"
        "                        }\n"
    ),
    (
        "                        if let Some(cb) = self.on_frame_cb {\n"
        "                            self.deliver_script_frame_cb(cb, msg.timestamp, &data_payload);\n"
        "                        }\n"
    ),
)

replace_once(
    "src/client/mod.rs",
    (
        "                    msg_dispatch::RTMP_MSG_AMF0_DATA => {\n"
        "                        self.frame_cb_scratch = tag_payload.to_vec();\n"
        "                        let frame = Frame {\n"
        "                            frame_type: FrameType::Script,\n"
        "                            timestamp: out_ts,\n"
        "                            size: self.frame_cb_scratch.len() as u32,\n"
        "                            data: self.frame_cb_scratch.as_ptr(),\n"
        "                            is_metadata: 1,\n"
        "                            ..Default::default()\n"
        "                        };\n"
        "                        cb(&frame);\n"
        "                    }\n"
    ),
    (
        "                    msg_dispatch::RTMP_MSG_AMF0_DATA => {\n"
        "                        self.deliver_script_frame_cb(cb, out_ts, tag_payload);\n"
        "                    }\n"
    ),
)

replace_once(
    "src/client/mod.rs",
    (
        "        populate_av_frame(&mut frame, &self.frame_cb_scratch);\n"
        "        cb(&frame);\n"
        "    }\n"
        "\n"
        "    fn queue_user_control_message(&mut self, payload: &[u8]) -> Result<()> {\n"
    ),
    (
        "        populate_av_frame(&mut frame, &self.frame_cb_scratch);\n"
        "        cb(&frame);\n"
        "    }\n"
        "\n"
        "    fn deliver_script_frame_cb(\n"
        "        &mut self,\n"
        "        cb: fn(&Frame),\n"
        "        timestamp: u32,\n"
        "        payload: &[u8],\n"
        "    ) {\n"
        "        let is_metadata = u8::from(is_on_metadata_payload(payload));\n"
        "        self.frame_cb_scratch.clear();\n"
        "        self.frame_cb_scratch.extend_from_slice(payload);\n"
        "        let frame = Frame {\n"
        "            frame_type: FrameType::Script,\n"
        "            timestamp,\n"
        "            size: self.frame_cb_scratch.len() as u32,\n"
        "            data: self.frame_cb_scratch.as_ptr(),\n"
        "            is_metadata,\n"
        "            ..Default::default()\n"
        "        };\n"
        "        cb(&frame);\n"
        "    }\n"
        "\n"
        "    fn queue_user_control_message(&mut self, payload: &[u8]) -> Result<()> {\n"
    ),
)

replace_once(
    "src/client/mod.rs",
    (
        "        assert_eq!(client.frame_cb_scratch.as_slice(), &video_payload[..]);\n"
        "    }\n"
        "\n"
        "    #[test]\n"
        "    fn drain_ready_messages_splits_multitrack_video() {\n"
    ),
    (
        "        assert_eq!(client.frame_cb_scratch.as_slice(), &video_payload[..]);\n"
        "    }\n"
        "\n"
        "    #[test]\n"
        "    fn script_callbacks_only_mark_on_metadata_events() {\n"
        "        use std::sync::{LazyLock, Mutex};\n"
        "\n"
        "        static FLAGS: LazyLock<Mutex<Vec<u8>>> = LazyLock::new(|| Mutex::new(Vec::new()));\n"
        "\n"
        "        let mut client = Client::new();\n"
        "        FLAGS.lock().unwrap().clear();\n"
        "\n"
        "        let mut cue_point = Buffer::new();\n"
        "        crate::amf::amf0::write_string(&mut cue_point, \"onCuePoint\").unwrap();\n"
        "        client.deliver_script_frame_cb(\n"
        "            |frame| FLAGS.lock().unwrap().push(frame.is_metadata),\n"
        "            10,\n"
        "            cue_point.as_slice(),\n"
        "        );\n"
        "\n"
        "        let mut metadata = Buffer::new();\n"
        "        crate::amf::amf0::write_string(&mut metadata, \"@setDataFrame\").unwrap();\n"
        "        crate::amf::amf0::write_string(&mut metadata, \"onMetaData\").unwrap();\n"
        "        client.deliver_script_frame_cb(\n"
        "            |frame| FLAGS.lock().unwrap().push(frame.is_metadata),\n"
        "            20,\n"
        "            metadata.as_slice(),\n"
        "        );\n"
        "\n"
        "        assert_eq!(*FLAGS.lock().unwrap(), vec![0, 1]);\n"
        "    }\n"
        "\n"
        "    #[test]\n"
        "    fn drain_ready_messages_splits_multitrack_video() {\n"
    ),
)

replace_once(
    "src/ertmp/connect_amf.rs",
    (
        "        Amf0Type::Number => {\n"
        "            *mask = amf0::read_number(buf)? as u32;\n"
        "            Ok(())\n"
        "        }\n"
    ),
    (
        "        Amf0Type::Number => {\n"
        "            let value = amf0::read_number(buf)?;\n"
        "            if !value.is_finite()\n"
        "                || value < 0.0\n"
        "                || value > u32::MAX as f64\n"
        "                || value.fract() != 0.0\n"
        "            {\n"
        "                return Err(ErrorCode::Amf);\n"
        "            }\n"
        "            *mask = value as u32;\n"
        "            Ok(())\n"
        "        }\n"
    ),
)

replace_once(
    "src/ertmp/connect_amf.rs",
    (
        "        read_caps_ex_amf(&mut buf, &mut parsed, &mut mask).unwrap();\n"
        "        assert_eq!(mask, CAPS_EX_MASK_MULTITRACK | CAPS_EX_MASK_MODEX);\n"
        "    }\n"
        "\n"
        "    #[test]\n"
        "    fn caps_ex_binary_round_trip() {\n"
    ),
    (
        "        read_caps_ex_amf(&mut buf, &mut parsed, &mut mask).unwrap();\n"
        "        assert_eq!(mask, CAPS_EX_MASK_MULTITRACK | CAPS_EX_MASK_MODEX);\n"
        "    }\n"
        "\n"
        "    #[test]\n"
        "    fn caps_ex_number_rejects_non_integral_or_out_of_range_values() {\n"
        "        for value in [\n"
        "            f64::NAN,\n"
        "            f64::INFINITY,\n"
        "            -1.0,\n"
        "            1.5,\n"
        "            u32::MAX as f64 + 1.0,\n"
        "        ] {\n"
        "            let mut buf = Buffer::new();\n"
        "            amf0::write_number(&mut buf, value).unwrap();\n"
        "            let mut parsed = CapsExit::default();\n"
        "            let mut mask = 0u32;\n"
        "            assert_eq!(\n"
        "                read_caps_ex_amf(&mut buf, &mut parsed, &mut mask),\n"
        "                Err(ErrorCode::Amf),\n"
        "                \"value {value:?} must be rejected\"\n"
        "            );\n"
        "        }\n"
        "    }\n"
        "\n"
        "    #[test]\n"
        "    fn caps_ex_binary_round_trip() {\n"
    ),
)

replace_once(
    "src/session/conn.rs",
    (
        "                let mut info = ConnectInfo::default();\n"
        "                command::read_connect(&mut buf, &mut info)?;\n"
        "                let app_len = info.app.iter().position(|&b| b == 0).unwrap_or(0);\n"
    ),
    (
        "                let mut info = ConnectInfo::default();\n"
        "                if command::read_connect(&mut buf, &mut info).is_err() {\n"
        "                    self.send_command_error(\n"
        "                        info.transaction_id,\n"
        "                        \"NetConnection.Connect.Rejected\",\n"
        "                        \"Invalid connect command or capability negotiation.\",\n"
        "                    )?;\n"
        "                    return Ok(());\n"
        "                }\n"
        "                let app_len = info.app.iter().position(|&b| b == 0).unwrap_or(0);\n"
    ),
)

replace_once(
    "src/session/conn.rs",
    (
        "    #[test]\n"
        "    fn connect_rejects_app_names_longer_than_routing_buffer() {\n"
        "        let mut conn = Conn::new();\n"
        "        let mut buf = Buffer::with_capacity(512);\n"
        "        let long_app = \"a\".repeat(256);\n"
        "        command::build_connect(\n"
        "            &mut buf,\n"
        "            &long_app,\n"
        "            \"rtmp://host/app\",\n"
        "            \"\",\n"
        "            \"\",\n"
        "            \"FMLE/3.0\",\n"
        "            0,\n"
        "            0,\n"
        "            None,\n"
        "        )\n"
        "        .unwrap();\n"
        "        assert_eq!(conn.handle_command(buf.as_slice()), Err(ErrorCode::Amf));\n"
        "        assert!(conn.app.is_empty());\n"
        "    }\n"
    ),
    (
        "    #[test]\n"
        "    fn connect_parse_failure_sends_error_response() {\n"
        "        let mut conn = Conn::new();\n"
        "        let mut buf = Buffer::with_capacity(512);\n"
        "        let long_app = \"a\".repeat(256);\n"
        "        command::build_connect(\n"
        "            &mut buf,\n"
        "            &long_app,\n"
        "            \"rtmp://host/app\",\n"
        "            \"\",\n"
        "            \"\",\n"
        "            \"FMLE/3.0\",\n"
        "            0,\n"
        "            0,\n"
        "            None,\n"
        "        )\n"
        "        .unwrap();\n"
        "        assert_eq!(conn.handle_command(buf.as_slice()), Ok(()));\n"
        "        assert!(conn.app.is_empty());\n"
        "        assert_ne!(conn.state, ConnState::AppConnected);\n"
        "        assert!(\n"
        "            conn.send_buffer\n"
        "                .peek()\n"
        "                .windows(b\"_error\".len())\n"
        "                .any(|window| window == b\"_error\")\n"
        "        );\n"
        "    }\n"
    ),
)
