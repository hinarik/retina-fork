#[cfg(test)]
mod fu_a_boundary_bug_tests {
    use bytes::BytesMut;
    use std::num::NonZeroU32;
    use crate::codec::h264::Depacketizer;

    /// 这个测试复现了 FU-A 分片边界的 bug
    /// 当一个分片以 00 结尾，下一个分片以 00 02 开始时，
    /// retina 会错误地认为这是 forbidden sequence 00 00 02
    #[test]
    fn test_fu_a_boundary_bug() {
        let mut depacketizer = Depacketizer::new(90_000, None).unwrap();
        
        let timestamp = crate::Timestamp {
            timestamp: 0,
            clock_rate: NonZeroU32::new(90_000).unwrap(),
            start: 0,
        };

        // 构造一个会被分成两个 FU-A 包的数据
        // 原始 NAL 数据: [header] ... X 00 00 02 ...
        // 这会被分成:
        // FU-A 1: ... X 00
        // FU-A 2: 00 02 ...
        
        // 第一个 FU-A 包 (start bit set)
        let mut fu_a_1 = BytesMut::new();
        fu_a_1.extend_from_slice(&[
            0x7c, // FU-A indicator (0111 1100)
            0x85, // FU header: S=1, E=0, R=0, Type=5 (1000 0101)
            // 一些数据，以 00 结尾
            0x01, 0x02, 0x03, 0x04, 0x05, 0x00
        ]);
        
        let pkt1 = crate::rtp::ReceivedPacketBuilder {
            ctx: crate::PacketContext::dummy(),
            stream_id: 0,
            timestamp,
            ssrc: 0x12345678,
            sequence_number: 1,
            loss: 0,
            mark: false,
            payload_type: 96,
        }
        .build(fu_a_1.freeze())
        .unwrap();
        
        // 这应该成功
        depacketizer.push(pkt1).unwrap();
        assert!(depacketizer.pull().is_none());
        
        // 第二个 FU-A 包 (end bit set)
        let mut fu_a_2 = BytesMut::new();
        fu_a_2.extend_from_slice(&[
            0x7c, // FU-A indicator
            0x45, // FU header: S=0, E=1, R=0, Type=5 (0100 0101)
            // 以 00 02 开始的数据
            0x00, 0x02, 0x06, 0x07, 0x08
        ]);
        
        let pkt2 = crate::rtp::ReceivedPacketBuilder {
            ctx: crate::PacketContext::dummy(),
            stream_id: 0,
            timestamp,
            ssrc: 0x12345678,
            sequence_number: 2,
            loss: 0,
            mark: true,
            payload_type: 96,
        }
        .build(fu_a_2.freeze())
        .unwrap();
        
        // 这里应该会触发 bug: "forbidden sequence 00 00 02 in NAL"
        let result = depacketizer.push(pkt2);
        
        // 期望的行为：这应该成功，因为 00 00 02 跨越了分片边界
        // 修复后：这应该成功，不再报告错误
        match result {
            Err(e) if e.contains("forbidden sequence 00 00 02") => {
                panic!("Bug still exists! Error: {}", e);
            }
            Ok(_) => {
                println!("Bug fixed! The packets were accepted successfully");
                // 验证我们能够成功获取重组后的 NAL
                let frame = depacketizer.pull();
                assert!(frame.is_some(), "Should have a frame after successful FU-A reassembly");
            }
            Err(e) => {
                panic!("Unexpected error: {}", e);
            }
        }
    }

    /// 这个测试展示了修复后的行为：FU-A 不再检查 forbidden sequences
    /// 因为 FU-A 传输的是已经验证过的 NAL 单元内容
    #[test]
    fn test_fu_a_no_longer_validates_content_during_reassembly() {
        let mut depacketizer = Depacketizer::new(90_000, None).unwrap();
        
        let timestamp = crate::Timestamp {
            timestamp: 0,
            clock_rate: NonZeroU32::new(90_000).unwrap(),
            start: 0,
        };

        // 第一个 FU-A 包（start bit set）
        let mut fu_a_1 = BytesMut::new();
        fu_a_1.extend_from_slice(&[
            0x7c, // FU-A indicator
            0x85, // FU header: S=1, E=0, R=0, Type=5
            // 包含 forbidden sequence 的数据
            0x01, 0x02, 0x00, 0x00, 0x02, 0x03
        ]);
        
        let pkt1 = crate::rtp::ReceivedPacketBuilder {
            ctx: crate::PacketContext::dummy(),
            stream_id: 0,
            timestamp,
            ssrc: 0x12345678,
            sequence_number: 1,
            loss: 0,
            mark: false,
            payload_type: 96,
        }
        .build(fu_a_1.freeze())
        .unwrap();
        
        // 第一个包应该成功（还没有完整的 NAL）
        depacketizer.push(pkt1).unwrap();
        
        // 第二个 FU-A 包（end bit set）
        let mut fu_a_2 = BytesMut::new();
        fu_a_2.extend_from_slice(&[
            0x7c, // FU-A indicator
            0x45, // FU header: S=0, E=1, R=0, Type=5
            0x04, 0x05 // 更多数据
        ]);
        
        let pkt2 = crate::rtp::ReceivedPacketBuilder {
            ctx: crate::PacketContext::dummy(),
            stream_id: 0,
            timestamp,
            ssrc: 0x12345678,
            sequence_number: 2,
            loss: 0,
            mark: true,
            payload_type: 96,
        }
        .build(fu_a_2.freeze())
        .unwrap();
        
        // 修复后：FU-A 不再检查 forbidden sequences
        // 即使 NAL 内容包含 00 00 02，也会被接受
        // 根据 RFC 6184，FU-A 的职责是忠实传输单个 NAL 单元的分片。
        // 对 NAL 单元内容的有效性校验（如检查 "forbidden sequences"）
        // 应该是下游 H.264 解码器的责任，而不是 RTP 解包器的。
        // 
        // 因此，即使我们构造一个理论上不合规的、包含 00 00 02 的 NAL 单元
        // 并将其分片，修复后的 Depacketizer 也应该能成功重组它而不会报错。
        let result = depacketizer.push(pkt2);
        assert!(result.is_ok());
        
        // 验证能够获取到帧
        let frame = depacketizer.pull();
        assert!(frame.is_some());
    }

    /// 测试更复杂的边界情况：trailing_zeros = 2
    #[test]
    fn test_fu_a_boundary_bug_with_two_trailing_zeros() {
        let mut depacketizer = Depacketizer::new(90_000, None).unwrap();
        
        let timestamp = crate::Timestamp {
            timestamp: 0,
            clock_rate: NonZeroU32::new(90_000).unwrap(),
            start: 0,
        };

        // 第一个包以 00 00 结尾
        let mut fu_a_1 = BytesMut::new();
        fu_a_1.extend_from_slice(&[
            0x7c, // FU-A indicator
            0x85, // FU header: S=1, E=0, R=0, Type=5
            0x01, 0x02, 0x03, 0x00, 0x00
        ]);
        
        let pkt1 = crate::rtp::ReceivedPacketBuilder {
            ctx: crate::PacketContext::dummy(),
            stream_id: 0,
            timestamp,
            ssrc: 0x12345678,
            sequence_number: 1,
            loss: 0,
            mark: false,
            payload_type: 96,
        }
        .build(fu_a_1.freeze())
        .unwrap();
        
        depacketizer.push(pkt1).unwrap();
        
        // 第二个包以 02 开始
        let mut fu_a_2 = BytesMut::new();
        fu_a_2.extend_from_slice(&[
            0x7c, // FU-A indicator
            0x45, // FU header: S=0, E=1, R=0, Type=5
            0x02, 0x03, 0x04
        ]);
        
        let pkt2 = crate::rtp::ReceivedPacketBuilder {
            ctx: crate::PacketContext::dummy(),
            stream_id: 0,
            timestamp,
            ssrc: 0x12345678,
            sequence_number: 2,
            loss: 0,
            mark: true,
            payload_type: 96,
        }
        .build(fu_a_2.freeze())
        .unwrap();
        
        // 这也会触发 bug
        let result = depacketizer.push(pkt2);
        match result {
            Err(e) if e.contains("forbidden sequence 00 00 02") => {
                panic!("Bug still exists with trailing_zeros=2! Error: {}", e);
            }
            Ok(_) => {
                println!("Bug fixed! The packets were accepted successfully (trailing_zeros=2 case)");
                let frame = depacketizer.pull();
                assert!(frame.is_some(), "Should have a frame after successful FU-A reassembly");
            }
            Err(e) => {
                panic!("Unexpected error: {}", e);
            }
        }
    }
}