#![feature(test)]

extern crate test;
use test::Bencher;
use std::iter::repeat;

use legion::*;

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct A(f32);

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct B(f32);

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct C(f32);

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct D(f32);

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct E(f32);

fn build(types: usize) -> World {
    let universe = Universe::new(None);
    let mut world = universe.create_world();

    let total = 1000000;
    let per_type = total / types;

    if types >= 1 {
        world.insert_from(
            (),
            repeat((A(0.0),)).take(per_type),
        );
    }

    if types >= 2 {
        world.insert_from(
            (),
            repeat((A(0.0), B(0.0))).take(per_type),
        );
    }

    if types >= 3 {
        world.insert_from(
            (),
            repeat((A(0.0), B(0.0), C(0.0))).take(per_type),
        );
    }

    if types >= 4 {
        world.insert_from(
            (),
            repeat((A(0.0), B(0.0), C(0.0), D(0.0))).take(per_type),
        );
    }

    if types >= 5 {
        world.insert_from(
            (),
            repeat((A(0.0), B(0.0), C(0.0), D(0.0), E(0.0))).take(per_type),
        );
    }

    world
}

#[bench]
fn bench_one_type(b: &mut Bencher) {
    b.iter(|| build(1));
}

#[bench]
fn bench_two_type(b: &mut Bencher) {
    b.iter(|| build(2));
}

#[bench]
fn bench_three_type(b: &mut Bencher) {
    b.iter(|| build(3));
}

#[bench]
fn bench_four_type(b: &mut Bencher) {
    b.iter(|| build(4));
}

#[bench]
fn bench_five_type(b: &mut Bencher) {
    b.iter(|| build(5));
}