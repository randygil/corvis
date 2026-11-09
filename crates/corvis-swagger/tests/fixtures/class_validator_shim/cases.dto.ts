// DTOs of the class-validator shim fixture. generate.mjs compiles this file with the OpenAPI compiler plugin.
import { ApiProperty } from '@nestjs/swagger';
import {
  ArrayMaxSize, ArrayMinSize, ArrayNotEmpty, ArrayUnique, Contains, IsDateString, IsEmail, IsEnum, IsHexColor,
  IsIn, IsInt, IsNegative, IsNotEmpty, IsOptional, IsPositive, IsString, IsUrl, IsUUID, Length, Matches, Max,
  MaxLength, Min, MinLength,
} from 'class-validator';

export enum Role {
  Admin = 'admin',
  User = 'user',
}

export class LengthRangeDto {
  @IsString() @MaxLength(50) name!: string;
  @IsInt() @Min(0) @Max(40) age!: number;
  @MinLength(2) short!: string;
}

export class LengthDto {
  @Length(2) onlyMin!: string;
  @Length(2, 20) both!: string;
}

export class FormatDto {
  @IsEmail() email!: string;
  @IsUrl() site!: string;
  @IsUUID() id!: string;
  @IsHexColor() color!: string;
}

export class PatternDto {
  @Matches(/^[a-z]+$/) slug!: string;
  @Contains('x') hasX!: string;
  @Contains('a.b') hasDot!: string;
}

export class SignDto {
  @IsPositive() count!: number;
  @IsNegative() debt!: number;
  @IsPositive() ratio!: number;
  @IsNegative() drop!: number;
}

export class AllowedDto {
  @IsIn(['persian', 'siamese']) breed!: string;
  @IsIn([1, 2, 3]) level!: number;
}

export class UnmappedDto {
  @IsInt() @IsNotEmpty() n!: number;
  @IsDateString() when!: string;
  @IsNotEmpty() text!: string;
}

export class ArraysDto {
  @MaxLength(10, { each: true }) @ArrayMaxSize(5) tags!: string[];
  @ArrayMinSize(1) @ArrayUnique() names!: string[];
  @ArrayNotEmpty() items!: string[];
  @IsIn(['a', 'b'], { each: true }) letters!: string[];
}

export class PrecedenceDto {
  @Min(5) @IsPositive() minThenPositive!: number;
  @Contains('x') @Matches(/^[a-z]+$/) containsThenMatches!: string;
  @MinLength(1) @Length(3, 9) minLengthThenLength!: string;
  @Max(10) @Max(20) repeated!: number;
}

export class OptionalDto {
  @IsOptional() @MaxLength(5) nick?: string;
  @IsOptional() @Min(1) level?: number;
}

export class ExplicitDto {
  @Max(40) @ApiProperty({ maximum: 30 }) capped!: number;
  @MaxLength(10) @ApiProperty({ description: 'Label' }) label!: string;
}

export class EnumDto {
  @IsEnum(Role) role!: Role;
}
