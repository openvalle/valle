use sha2::{Digest, Sha256};
use valle_motion::spring::{SpringParams, spring_sample_at};

pub fn hash() -> [u8; 32] {
    let mut hash = Sha256::new();
    for damping in [0.0, 8.0, 19.99999999, 20.0, 20.00000001, 40.0, 2000.0] {
        for initial_velocity in [-4.0, 0.0, 0.5, 6.0] {
            for t in [
                -1.0,
                0.0,
                0.000001,
                0.001,
                1.0 / 24.0,
                1.0 / 30.0,
                1.0 / 60.0,
                0.2,
                0.6,
                2.0,
                8.0,
                60.0,
            ] {
                let s = spring_sample_at(
                    t,
                    SpringParams {
                        mass: 1.0,
                        stiffness: 100.0,
                        damping,
                        initial_velocity,
                    },
                );
                hash.update(s.position.to_bits().to_le_bytes());
                hash.update(s.velocity.to_bits().to_le_bytes());
            }
        }
    }
    hash.finalize().into()
}
